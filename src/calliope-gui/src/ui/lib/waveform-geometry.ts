export function columns(peaks: number[], width: number): number[] {
  if (width <= 0 || peaks.length === 0) {
    return [];
  }

  const result: number[] = [];
  for (let x = 0; x < width; x++) {
    const a = Math.floor(x * peaks.length / width);
    const b = Math.max(a + 1, Math.floor((x + 1) * peaks.length / width));
    const maxPeak = Math.max(...peaks.slice(a, Math.min(b, peaks.length)));
    result.push(maxPeak);
  }
  return result;
}

export function xOf(ms: number, durationMs: number, width: number): number {
  if (durationMs <= 0) {
    return 0;
  }
  return Math.round(Math.min(Math.max(ms, 0), durationMs) / durationMs * (width - 1));
}

export function loopMarks(loops: {start_ms: number; end_ms: number | null}[], durationMs: number, width: number): {x1: number; x2: number | null}[] {
  return loops.map(loop => {
    const x1 = xOf(loop.start_ms, durationMs, width);
    const x2 = loop.end_ms === null ? null : xOf(loop.end_ms, durationMs, width);
    return { x1, x2 };
  });
}
/** Fine steps per beat and the largest offset from `detected_ms`; same as `tempo_map.rs`. */
export const FINE_STEPS_PER_BEAT = 64;
export const MAX_OFFSET_STEPS = 256;

/** One fine step (1/64 beat) at `bpm`, in ms. Mirrors Rust `fine_ms`. */
export function fineMs(bpm: number): number {
  return 60000 / (bpm * FINE_STEPS_PER_BEAT);
}

export interface SectionMark {
  /** null for the first section: its label sits at the left edge, no bar */
  x: number | null;
  bpm: number;
  index: number;
  detected: boolean;
}

export function sectionMarks(
  segments: { start_ms: number; bpm: number; detected_ms?: number | null }[],
  durationMs: number,
  width: number,
): SectionMark[] {
  return segments.map((s, index) => ({
    x: index === 0 ? null : xOf(s.start_ms, durationMs, width),
    bpm: s.bpm,
    index,
    detected: s.detected_ms != null,
  }));
}

/** Whether a label of `textWidth` px at `x` ends before the next bar (`nextX`, null = none). */
export function labelFits(x: number, nextX: number | null, textWidth: number): boolean {
  return nextX === null || x + textWidth <= nextX;
}

/** Rust `f64::round`: halves away from zero. */
function roundHalfAway(v: number): number {
  return Math.sign(v) * Math.round(Math.abs(v));
}

/**
 * The offset (fine steps from `detectedMs`) for a pointer at `pointerMs`: rounded, clamped to
 * +-`limit` (default MAX_OFFSET_STEPS, Infinity allowed) and so the change stays within [minMs, maxMs] (one beat from its
 * neighbours; use +-Infinity for none). Mirrors `TempoMap::set_change_offset`; 0 if no room.
 */
export function dragSteps(detectedMs: number, pointerMs: number, leftBpm: number, minMs: number, maxMs: number, limit = MAX_OFFSET_STEPS): number {
  const fine = fineMs(leftBpm);
  const at = (n: number) => detectedMs + n * fine;
  let min = Math.max(-limit, Math.ceil(Math.max((minMs - detectedMs) / fine, -1e9)));
  while (at(min) < minMs) {
    min++;
  }
  let max = limit;
  if (Number.isFinite(maxMs)) {
    max = Math.min(max, Math.floor(Math.min((maxMs - detectedMs) / fine, 1e9)));
    while (at(max) > maxMs) {
      max--;
    }
  }
  if (min > max) {
    return 0;
  }
  const steps = roundHalfAway((pointerMs - detectedMs) / fine);
  return Math.min(Math.max(steps, min), max);
}

/** The absolute offset in fine steps of a change at `startMs` from `detectedMs`. */
export function offsetSteps(startMs: number, detectedMs: number, leftBpm: number): number {
  return roundHalfAway((startMs - detectedMs) / fineMs(leftBpm));
}

type MoveSeg = { start_ms: number; bpm: number; detected_ms?: number | null };

/** The point a change's offset is measured from: `detected_ms`, else the left section's start. Needs i >= 1. */
export function moveRefMs(segs: MoveSeg[], i: number): number {
  return segs[i].detected_ms ?? segs[i - 1].start_ms;
}

/** Largest |offset| in fine steps: detected changes keep max(256, current offset); others are free. */
export function moveLimit(segs: MoveSeg[], i: number): number {
  const d = segs[i].detected_ms;
  if (d == null) {
    return Infinity;
  }
  return Math.max(MAX_OFFSET_STEPS, Math.abs(offsetSteps(segs[i].start_ms, d, segs[i - 1].bpm)));
}
