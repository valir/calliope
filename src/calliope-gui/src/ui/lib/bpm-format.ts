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
