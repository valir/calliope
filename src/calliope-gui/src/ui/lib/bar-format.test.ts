import { describe, expect, it } from 'vitest';
import { formatBar, formatLoopTime, parseBar, parseLoopTime } from './bar-format';

describe('formatBar', () => {
  it.each([
    [0, 4, '1.1.0'], [64, 4, '2.1.0'], [72, 4, '2.1.8'], [16, 4, '1.2.0'],
    [-16, 4, '0.4.0'], [48, 3, '2.1.0'],
  ])('%d/%d -> %s', (t, b, e) => expect(formatBar(t, b)).toBe(e));
});

describe('parseBar', () => {
  it.each([['2.1.8', 4, 72], ['1.1', 4, 0], ['0.4.0', 4, -16]])('%s', (s, b, e) =>
    expect(parseBar(s, b)).toBe(e));
  it.each(['1.5.0', '1.1.16', 'a.b', ''])('rejects %j', (s) => expect(parseBar(s, 4)).toBeNull());
});

describe('formatLoopTime', () => {
  it.each([[0, '0:00.000'], [67250, '1:07.250'], [13166.667, '0:13.167'], [-3, '0:00.000']])(
    '%d', (ms, e) => expect(formatLoopTime(ms)).toBe(e));
});

describe('parseLoopTime', () => {
  it.each([['1:07.250', 67250], ['1:07.25', 67250], ['1:07', 67000], ['75.5', 75500]])('%s', (s, e) =>
    expect(parseLoopTime(s)).toBe(e));
  it.each(['1:7', 'x'])('rejects %j', (s) => expect(parseLoopTime(s)).toBeNull());
});
