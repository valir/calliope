import type { TempoSegment } from './ipc';

/** Mirrors the Rust limit in `set_section_name` (characters, not bytes). */
export const MAX_SECTION_NAME = 40;

/** The section's metronome; a missing key means on (files older than schema 4). */
export function sectionClicks(seg: TempoSegment): boolean {
  return seg.metronome !== false;
}

/** The section's count-in; a missing key means off. */
export function sectionCountIn(seg: TempoSegment): boolean {
  return seg.count_in === true;
}

/** The name as Rust accepts it: control characters removed, trimmed, at most MAX_SECTION_NAME characters. */
export function cleanSectionName(raw: string): string {
  const kept = Array.from(raw).filter((c) => {
    const n = c.codePointAt(0) ?? 0;
    return n >= 0x20 && !(n >= 0x7f && n <= 0x9f);
  });
  return Array.from(kept.join('').trim()).slice(0, MAX_SECTION_NAME).join('').trim();
}

/** Parses an offset string into a number of steps. */
export function parseOffset(text: string): number | null {
  const t = text.trim().replace(/−/g, '-');
  if (!/^[+-]?\d+$/.test(t)) {
    return null;
  }
  const n = Math.max(-1e9, Math.min(1e9, Number(t)));
  return n === 0 ? 0 : n;
}

/** Formats a number of steps as an offset string. */
export function formatOffset(steps: number): string {
  return steps > 0 ? `+${steps}` : `${steps}`;
}

/** Formats a duration in milliseconds as an offset time string. */
export function offsetTime(ms: number): string {
  const sign = ms < 0 ? '-' : '+';
  const total = Math.abs(ms) / 1000;
  const m = Math.floor(total / 60);
  const s = (total - m * 60).toFixed(3);
  return `${sign}${m}:${Number(s) < 10 ? '0' : ''}${s}`;
}
