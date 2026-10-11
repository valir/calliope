import { parsePosition } from './time-format';

export function formatBar(tick: number, beatsPerBar: number): string {
  const perBar = 16 * beatsPerBar;
  const bar = Math.floor(tick / perBar) + 1;
  const rest = tick - (bar - 1) * perBar;
  const beat = Math.floor(rest / 16) + 1;
  const sub = rest % 16;
  return `${bar}.${beat}.${sub}`;
}

export function parseBar(text: string, beatsPerBar: number): number | null {
  const m = /^(-?\d+)\.(\d+)(?:\.(\d+))?$/.exec(text.trim());
  if (!m) return null;
  const bar = Number(m[1]);
  const beat = Number(m[2]);
  const sub = m[3] === undefined ? 0 : Number(m[3]);
  if (beat < 1 || beat > beatsPerBar || sub < 0 || sub > 15) return null;
  return (bar - 1) * 16 * beatsPerBar + (beat - 1) * 16 + sub;
}

export function formatLoopTime(ms: number): string {
  const r = Math.round(Math.max(ms, 0));
  const m = Math.floor(r / 60000);
  const s = Math.floor(r / 1000) % 60;
  const f = r % 1000;
  return `${m}:${String(s).padStart(2, '0')}.${String(f).padStart(3, '0')}`;
}

export function parseLoopTime(text: string): number | null {
  const t = text.trim();
  const m = /^(\d+):([0-5]\d)(?:\.(\d{1,3}))?$/.exec(t);
  if (m) {
    const frac = m[3] === undefined ? 0 : Number(m[3].padEnd(3, '0'));
    return (Number(m[1]) * 60 + Number(m[2])) * 1000 + frac;
  }
  return parsePosition(t);
}
