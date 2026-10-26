import { describe, expect, it } from 'vitest';
import {
  cleanSectionName,
  MAX_SECTION_NAME,
  sectionClicks,
  sectionCountIn,
  parseOffset,
  formatOffset,
  offsetTime
} from './section-meta';

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

  describe('offset helpers', () => {
    it('parses offsets', () => {
      expect(parseOffset('+10')).toBe(10);
      expect(parseOffset('10')).toBe(10);
      expect(parseOffset(' -8 ')).toBe(-8);
      expect(parseOffset('−8')).toBe(-8);
      expect(parseOffset('0')).toBe(0);
      expect(parseOffset('-0')).toBe(0);
      expect(Object.is(parseOffset('-0'), 0)).toBe(true);
      expect(parseOffset('1.5')).toBeNull();
      expect(parseOffset('abc')).toBeNull();
      expect(parseOffset('')).toBeNull();
      expect(parseOffset('+')).toBeNull();
      expect(parseOffset('1e3')).toBeNull();
      expect(parseOffset('99999999999999999999')).toBe(1e9);
      expect(parseOffset('-99999999999999999999')).toBe(-1e9);
    });

    it('formats offsets', () => {
      expect(formatOffset(10)).toBe('+10');
      expect(formatOffset(0)).toBe('0');
      expect(formatOffset(-8)).toBe('-8');
    });

    it('offsets time', () => {
      expect(offsetTime(78.125)).toBe('+0:00.078');
      expect(offsetTime(-62.5)).toBe('-0:00.063');
      expect(offsetTime(0)).toBe('+0:00.000');
      expect(offsetTime(-1200)).toBe('-0:01.200');
      expect(offsetTime(65000)).toBe('+1:05.000');
    });
  });
});
