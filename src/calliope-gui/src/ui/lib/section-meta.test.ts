import { describe, expect, it } from 'vitest';
import { cleanSectionName, MAX_SECTION_NAME, sectionClicks, sectionCountIn } from './section-meta';

describe('section meta', () => {
  it('reads the flags with their defaults', () => {
    expect(sectionClicks({ start_ms: 0, bpm: 120 })).toBe(true);
    expect(sectionClicks({ start_ms: 0, bpm: 120, metronome: false })).toBe(false);
    expect(sectionCountIn({ start_ms: 0, bpm: 120 })).toBe(false);
    expect(sectionCountIn({ start_ms: 0, bpm: 120, count_in: true })).toBe(true);
  });
  it('cleans names like Rust does', () => {
    expect(cleanSectionName('  Verse ')).toBe('Verse');
    expect(cleanSectionName('Ch\torus\n')).toBe('Chorus');
    expect(cleanSectionName('')).toBe('');
    expect(cleanSectionName('x'.repeat(50))).toHaveLength(MAX_SECTION_NAME);
    expect(cleanSectionName('é'.repeat(41))).toBe('é'.repeat(40));
  });
});
