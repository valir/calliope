import { describe, it, expect } from 'vitest';
import { columns, xOf, loopMarks, fineMs, sectionMarks, labelFits, dragSteps, offsetSteps } from './waveform-geometry';

describe('columns', () => {
  it('should handle empty peaks', () => {
    expect(columns([], 10)).toEqual([]);
  });

  it('should calculate columns correctly', () => {
    expect(columns([0.1, 0.5, 0.2, 0.9], 2)).toEqual([0.5, 0.9]);
    expect(columns([0.1, 0.5], 4)).toEqual([0.1, 0.1, 0.5, 0.5]);
  });
});

describe('xOf', () => {
  it('should handle invalid duration', () => {
    expect(xOf(5000, 0, 801)).toEqual(0);
  });

  it('should calculate x position correctly', () => {
    expect(xOf(5000, 20000, 801)).toEqual(200);
    expect(xOf(-1, 20000, 801)).toEqual(0);
    expect(xOf(30000, 20000, 801)).toEqual(800);
  });
});

describe('loopMarks', () => {
  it('should calculate loop marks correctly', () => {
    expect(loopMarks([{start_ms: 2500, end_ms: 6500}, {start_ms: 18500, end_ms: null}], 20000, 801)).toEqual([
      {x1: 100, x2: 260},
      {x1: 740, x2: null}
    ]);
  });
});
// Numbers of tempo_map.rs tests: 120 BPM first section anchored at 500 ms, detected change 90 BPM.
describe('fineMs', () => {
  it('should be 1/64 beat', () => {
    expect(fineMs(120)).toBe(7.8125);
  });
});

describe('sectionMarks', () => {
  it('should mark changes and flag detected ones', () => {
    const marks = sectionMarks(
      [{ start_ms: 500, bpm: 120 }, { start_ms: 10300, bpm: 90, detected_ms: 10300 }, { start_ms: 30000, bpm: 100, detected_ms: null }],
      40000, 401);
    expect(marks).toEqual([
      { x: null, bpm: 120, index: 0, detected: false },
      { x: 103, bpm: 90, index: 1, detected: true },
      { x: 300, bpm: 100, index: 2, detected: false },
    ]);
  });
});

describe('labelFits', () => {
  it('should compare with the next bar', () => {
    expect(labelFits(10, 60, 50)).toBe(true);
    expect(labelFits(10, 59, 50)).toBe(false);
    expect(labelFits(10, null, 500)).toBe(true);
  });
});

describe('dragSteps', () => {
  const d = 10300;
  const far = (a: number) => [a - 1e6, a + 1e6] as const;
  it('should round to whole steps from detected_ms', () => {
    const [lo, hi] = far(d);
    for (const s of [0, 1, 3, -5, 256]) {
      expect(dragSteps(d, d + s * 7.8125, 120, lo, hi)).toBe(s);
    }
    expect(dragSteps(d, d + 2.4 * 7.8125, 120, lo, hi)).toBe(2);
    expect(dragSteps(d, d + 2.6 * 7.8125, 120, lo, hi)).toBe(3);
    expect(dragSteps(d, d - 2.5 * 7.8125, 120, lo, hi)).toBe(-3);
  });
  it('should clamp to +-256', () => {
    const [lo, hi] = far(d);
    expect(dragSteps(d, d + 1e5, 120, lo, hi)).toBe(256);
    expect(dragSteps(d, d - 1e5, 120, lo, hi)).toBe(-256);
  });
  it('should clamp to the neighbour limits like Rust', () => {
    // Rust test: detected 29000, next change at 29900 (90 BPM change keeps 1 beat of room)
    const hi = 29900 - 60000 / 90;
    const s = dragSteps(29000, 1e9, 120, 0, hi);
    expect(s).toBeLessThan(100);
    expect(29000 + s * 7.8125).toBeLessThanOrEqual(hi);
    expect(hi - (29000 + s * 7.8125)).toBeLessThan(7.8125);
    // lower limit: one beat of 120 BPM after the 500 ms anchor
    const l = dragSteps(1100, -1e9, 120, 1000, Infinity);
    expect(l).toBeGreaterThan(-256);
    expect(1100 + l * 7.8125).toBeGreaterThanOrEqual(1000);
    expect(1100 + l * 7.8125 - 1000).toBeLessThan(7.8125);
  });
});

describe('offsetSteps', () => {
  it('should invert the offset', () => {
    for (const s of [0, 1, 3, -5, 256]) {
      expect(offsetSteps(10300 + s * 7.8125, 10300, 120)).toBe(s);
    }
  });
});
