import { describe, expect, it } from 'vitest';
import { formatBpm, parseBpm, scaleBpm } from './bpm-format';

describe('formatBpm', () => {
  it.each([[0, '0'], [120, '120'], [92.5, '92.5']])('%d', (v, e) => expect(formatBpm(v)).toBe(e));
});

describe('parseBpm', () => {
  it.each([['120', 120], ['92.5', 92.5], ['0', 0]])('%s', (s, e) => expect(parseBpm(s)).toBe(e));
  it.each(['19.9', '300.1', '12a', '92.55', ''])('rejects %j', (s) => expect(parseBpm(s)).toBeNull());
});

describe('scaleBpm', () => {
  it('doubles and halves with one decimal', () => {
    expect(scaleBpm(93, 2)).toBe(186);
    expect(scaleBpm(186, 0.5)).toBe(93);
    expect(scaleBpm(93.5, 0.5)).toBe(46.8);
    expect(scaleBpm(150, 2)).toBe(300);
    expect(scaleBpm(40, 0.5)).toBe(20);
  });
  it('refuses results outside 20..300', () => {
    expect(scaleBpm(160, 2)).toBeNull();
    expect(scaleBpm(39.8, 0.5)).toBeNull();
  });
});
