import { describe, it, expect } from 'vitest';
import { columns, xOf, loopMarks, fineMs, sectionMarks, labelFits, dragSteps, offsetSteps, moveRefMs, moveLimit, playStartMs, stepBpm, firstBeatLimits, firstBeatSteps, firstBeatAt } from './waveform-geometry';

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
      { x: 5, bpm: 120, index: 0, detected: false },
      { x: 103, bpm: 90, index: 1, detected: true },
      { x: 300, bpm: 100, index: 2, detected: false },
    ]);
    expect(sectionMarks([{ start_ms: 0, bpm: 120 }], 40000, 401)[0].x).toBe(0);
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

describe('free moves', () => {
  it('should reference the detected point or the left start', () => {
    const segs = [
      { start_ms: 0, bpm: 120 },
      { start_ms: 4000, bpm: 90 },
      { start_ms: 9000, bpm: 100, detected_ms: 8800 },
    ];
    expect(moveRefMs(segs, 1)).toBe(0);
    expect(moveRefMs(segs, 2)).toBe(8800);
  });
  it('should limit only detected changes, widened to the current offset', () => {
    const f = 60000 / (120 * 64);
    const segs = [
      { start_ms: 500, bpm: 120 },
      { start_ms: 5000, bpm: 90 },
      { start_ms: 10000 + 400 * f, bpm: 100, detected_ms: 10000 },
      { start_ms: 20000 + 100 * f, bpm: 100, detected_ms: 20000 },
    ];
    expect(moveLimit(segs, 1)).toBe(Infinity);
    expect(moveLimit(segs, 3)).toBe(256);
    const wide = [segs[0], { ...segs[1], bpm: 120 }, segs[2]];
    expect(moveLimit(wide, 2)).toBe(400);
  });
  it('should move a placed change freely on the left grid', () => {
    const f = 60000 / (120 * 64);
    expect(dragSteps(500, 500 + 1000 * f, 120, 0, Infinity, Infinity)).toBe(1000);
    expect(dragSteps(500, 500 - 30 * f, 120, 0, Infinity, Infinity)).toBe(-30);
    expect(dragSteps(500, -1e9, 120, 0, Infinity, Infinity)).toBe(-64);
    expect(dragSteps(500, 1e9, 120, 0, 500 + 5000 * f, Infinity)).toBe(5000);
  });
  it('should accept 400 and refuse 401 for a detected change widened to 400', () => {
    const f = 60000 / (120 * 64);
    expect(dragSteps(10000, 10000 + 399 * f, 120, 0, 1e9, 400)).toBe(399);
    expect(dragSteps(10000, 10000 + 400 * f, 120, 0, 1e9, 400)).toBe(400);
    expect(dragSteps(10000, 10000 + 401 * f, 120, 0, 1e9, 400)).toBe(400);
  });
});

describe('first beat', () => {
  const set = [{ start_ms: 1200, bpm: 120, found_ms: 1200 }, { start_ms: 2600, bpm: 90 }];
  const unset = [{ start_ms: 1200, bpm: 120 }];

  it('playStartMs follows found_ms', () => {
    expect(playStartMs(set)).toBe(1200);
    expect(playStartMs(unset)).toBe(0);
    expect(playStartMs([])).toBe(0);
  });

  it('stepBpm and moveRefMs/moveLimit for bar 0', () => {
    expect(stepBpm(set, 0)).toBe(120);
    expect(stepBpm(set, 1)).toBe(120);
    expect(moveRefMs(set, 0)).toBe(1200);
    expect(moveRefMs([{ start_ms: 900, bpm: 120, found_ms: null }], 0)).toBe(900);
    expect(moveLimit(set, 0)).toBe(Infinity);
  });

  it('firstBeatLimits with and without a second section', () => {
    expect(firstBeatLimits(set, 5000)).toEqual([0, 2100]);
    expect(firstBeatLimits(unset, 3000)).toEqual([0, 2500]);
  });

  it('firstBeatSteps and firstBeatAt clamp', () => {
    expect(firstBeatSteps(1200, 1207.9, 120, 0, 2500)).toBe(1);
    expect(firstBeatSteps(1200, -50, 120, 0, 2500)).toBe(-154);
    expect(firstBeatAt(1200, -154, 120, 0, 2500)).toBe(0);
    expect(firstBeatSteps(1200, 9999, 120, 0, 2500)).toBe(167);
    expect(firstBeatAt(1200, 167, 120, 0, 2500)).toBe(2500);
  });

  it('firstBeatSteps and firstBeatAt hold at lo when hi < lo', () => {
    expect(firstBeatAt(1200, 5, 120, 0, -100)).toBe(0);
    expect(firstBeatAt(1200, -500, 120, 0, -100)).toBe(0);
    expect(firstBeatSteps(1200, 900, 120, 0, -100)).toBe(firstBeatSteps(1200, 900, 120, 0, 0));
    expect(firstBeatAt(1200, firstBeatSteps(1200, 900, 120, 0, -100), 120, 0, -100)).toBeGreaterThanOrEqual(0);
  });
});
