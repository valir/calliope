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

/** Converts milliseconds to m:ss.s format. */
export function pieceTime(ms: number): string {
  const t = Math.round(Math.max(0, ms) / 100) / 10;
  const m = Math.floor(t / 60);
  const s = (t - m * 60).toFixed(1);
  return `${m}:${Number(s) < 10 ? '0' : ''}${s}`;
}

/** Returns a status message about BPM detection. */
export function detectStatus(r: { old_bpm: number; new_bpm: number | null; from_ms: number; to_ms: number; hint: 'double' | 'half' | null }): string {
  const a = r.old_bpm.toFixed(1);
  const b = (r.new_bpm ?? r.old_bpm).toFixed(1);
  const t = `${pieceTime(r.from_ms)} – ${pieceTime(r.to_ms)}`;
  if (r.hint === 'double') return `BPM ${a} → ${b} from ${t}, about twice: try ÷2`;
  if (r.hint === 'half') return `BPM ${a} → ${b} from ${t}, about half: try ×2`;
  return `BPM ${a} → ${b}, detected from ${t}`;
}

/** Returns a running status message. */
export function detectRunning(v: { from_ms: number; to_ms: number; phase: string }): string {
  const t = `${pieceTime(v.from_ms)} – ${pieceTime(v.to_ms)}`;
  if (v.phase === 'waiting') return `Detecting the BPM from ${t} (waiting for the edge-AI server)...`;
  return `Detecting the BPM from ${t}...`;
}
