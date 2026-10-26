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
  /** Pixel column of the section's start; never null now (the first bar is drawn too) */
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
    x: xOf(s.start_ms, durationMs, width),
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

type MoveSeg = { start_ms: number; bpm: number; detected_ms?: number | null; found_ms?: number | null };

/** The point a bar's offset is measured from: for index 0 `found_ms` (else its start); else `detected_ms`, else the left section's start. */
export function moveRefMs(segs: MoveSeg[], i: number): number {
  if (i === 0) {
    return segs[0].found_ms ?? segs[0].start_ms;
  }
  return segs[i].detected_ms ?? segs[i - 1].start_ms;
}

/** Largest |offset| in fine steps: the first bar is free; detected changes keep max(256, current offset); others are free. */
export function moveLimit(segs: MoveSeg[], i: number): number {
  if (i === 0) {
    return Infinity;
  }
  const d = segs[i].detected_ms;
  if (d == null) {
    return Infinity;
  }
  return Math.max(MAX_OFFSET_STEPS, Math.abs(offsetSteps(segs[i].start_ms, d, segs[i - 1].bpm)));
}

/** Where playback starts: the first beat when it is set (`found_ms` present), else 0. */
export function playStartMs(segs: MoveSeg[]): number {
  if (segs.length === 0) {
    return 0;
  }
  return segs[0].found_ms != null ? Math.max(0, segs[0].start_ms) : 0;
}

/** The BPM whose 1/64 beat moves bar `i`: the first section's for bar 0, else the section to its left. */
export function stepBpm(segs: MoveSeg[], i: number): number {
  return i === 0 ? segs[0].bpm : segs[i - 1].bpm;
}

/** Range `[lo, hi]` of the first beat: 0 up to one beat before the second section (or the end). */
export function firstBeatLimits(segs: MoveSeg[], durationMs: number): [number, number] {
  return [0, (segs[1]?.start_ms ?? durationMs) - 60000 / segs[0].bpm];
}

/** Fine steps of the first beat from `refMs` for a pointer at `pointerMs`; mirrors the Rust clamp (floor/ceil). */
export function firstBeatSteps(refMs: number, pointerMs: number, bpm: number, lo: number, hi: number): number {
  const fine = fineMs(bpm);
  const nmin = Math.floor((lo - refMs) / fine);
  const nmax = Number.isFinite(hi) ? Math.ceil((Math.max(hi, lo) - refMs) / fine) : Infinity;
  return Math.min(Math.max(roundHalfAway((pointerMs - refMs) / fine), nmin), nmax);
}

/** The first beat's position for `steps` fine steps from `refMs`, clamped to `[lo, hi]`. */
export function firstBeatAt(refMs: number, steps: number, bpm: number, lo: number, hi: number): number {
  return Math.min(Math.max(refMs + steps * fineMs(bpm), lo), Math.max(hi, lo));
}

/** How long the keyboard readout stays after the last key press, in ms. */
export const KEY_READOUT_MS = 1500;

/** The offset of bar `i` from its reference (the drag readout's reference): fine steps and the applied ms. */
export function sectionOffset(segs: MoveSeg[], i: number): { steps: number; ms: number } {
  const ref = moveRefMs(segs, i);
  return { steps: offsetSteps(segs[i].start_ms, ref, stepBpm(segs, i)), ms: segs[i].start_ms - ref };
}

/** The drag clamp for a wanted step count of bar `i`; mirrors `TempoMap::move_first_beat` / `move_change`. */
export function clampMoveSteps(segs: MoveSeg[], i: number, steps: number, durationMs: number): number {
  const ref = moveRefMs(segs, i);
  if (i === 0) {
    const [lo, hi] = firstBeatLimits(segs, durationMs);
    return firstBeatSteps(ref, ref + steps * fineMs(segs[0].bpm), segs[0].bpm, lo, hi);
  }
  const prev = segs[i - 1];
  const next = segs[i + 1];
  const lo = prev.start_ms + 60000 / prev.bpm;
  const hi = (next ? next.start_ms : durationMs) - 60000 / segs[i].bpm;
  return dragSteps(ref, ref + steps * fineMs(prev.bpm), prev.bpm, lo, hi, moveLimit(segs, i));
}
