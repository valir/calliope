import { describe, it, expect } from 'vitest';
import { columns, xOf, loopMarks } from './waveform-geometry';

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