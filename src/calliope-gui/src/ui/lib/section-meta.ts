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
