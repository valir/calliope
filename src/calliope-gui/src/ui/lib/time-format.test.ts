import { describe, it, expect } from 'vitest';
import { formatPosition, parsePosition } from './time-format';

describe('time-format', () => {
  describe('formatPosition', () => {
    it('formats various milliseconds correctly', () => {
      expect(formatPosition(0)).toBe('0:00.0');
      expect(formatPosition(99)).toBe('0:00.0');
      expect(formatPosition(100)).toBe('0:00.1');
      expect(formatPosition(7400)).toBe('0:07.4');
      expect(formatPosition(187400)).toBe('3:07.4');
      expect(formatPosition(600000)).toBe('10:00.0');
      expect(formatPosition(-5)).toBe('0:00.0');
    });
  });

  describe('parsePosition', () => {
    const good = {
      '3:07.4': 187400,
      '3:07:4': 187400,
      '3:07': 187000,
      '0:00.0': 0,
      ' 1:02.3 ': 62300,
      '75': 75000,
      '75.5': 75500,
    };
    Object.entries(good).forEach(([input, expected]) => {
      it(`parses "${input}" to ${expected}`, () => {
        expect(parsePosition(input)).toBe(expected);
      });
    });

    const bad = [
      '3:7.4',
      '3:60.0',
      '3:07.45',
      'abc',
      '',
      '-1',
      '1:02.',
    ];
    bad.forEach((input) => {
      it(`returns null for invalid input "${input}"`, () => {
        expect(parsePosition(input)).toBeNull();
      });
    });
  });
});
