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