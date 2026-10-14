export function formatBpm(bpm: number): string {
  if (bpm === 0) return '0';
  return Number.isInteger(bpm) ? String(bpm) : bpm.toFixed(1);
}

export function parseBpm(text: string): number | null {
  const t = text.trim();
  if (!/^\d+(?:\.\d)?$/.test(t)) return null;
  const v = Number(t);
  if (v === 0 || (v >= 20 && v <= 300)) return v;
  return null;
}

/** Doubles or halves a BPM, rounded to one decimal; null when the result is outside 20..300. Mirrors Rust `scale_section`. */
export function scaleBpm(bpm: number, factor: 2 | 0.5): number | null {
  const v = Math.round(bpm * factor * 10) / 10;
  return v >= 20 && v <= 300 ? v : null;
}
