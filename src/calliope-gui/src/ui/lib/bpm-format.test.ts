import { describe, expect, it } from 'vitest';
import { formatBpm, parseBpm } from './bpm-format';

describe('formatBpm', () => {
  it.each([[0, '0'], [120, '120'], [92.5, '92.5']])('%d', (v, e) => expect(formatBpm(v)).toBe(e));
});

describe('parseBpm', () => {
  it.each([['120', 120], ['92.5', 92.5], ['0', 0]])('%s', (s, e) => expect(parseBpm(s)).toBe(e));
  it.each(['19.9', '300.1', '12a', '92.55', ''])('rejects %j', (s) => expect(parseBpm(s)).toBeNull());
});
