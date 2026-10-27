import { describe, expect, it } from 'vitest';
import { formatBpm, parseBpm, scaleBpm, pieceTime, detectStatus, detectRunning } from './bpm-format';

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

describe('pieceTime', () => {
  it.each([
    [10500, '0:10.5'],
    [17700, '0:17.7'],
    [62000, '1:02.0'],
    [59960, '1:00.0'],
    [0, '0:00.0'],
  ])('%p → %s', (ms, expected) => {
    expect(pieceTime(ms)).toBe(expected);
  });
});

describe('detectStatus', () => {
  it('detects status without hint', () => {
    expect(detectStatus({ old_bpm: 92, new_bpm: 93.4, from_ms: 41200, to_ms: 47100, hint: null }))
      .toBe('BPM 92.0 → 93.4, detected from 0:41.2 – 0:47.1');
  });

  it('detects status with hints', () => {
    expect(detectStatus({ old_bpm: 100, new_bpm: 198.4, from_ms: 10500, to_ms: 17700, hint: 'double' }))
      .toBe('BPM 100.0 → 198.4 from 0:10.5 – 0:17.7, about twice: try ÷2');
    expect(detectStatus({ old_bpm: 100, new_bpm: 198.4, from_ms: 10500, to_ms: 17700, hint: 'half' }))
      .toBe('BPM 100.0 → 198.4 from 0:10.5 – 0:17.7, about half: try ×2');
  });

  it('handles null new_bpm', () => {
    expect(detectStatus({ old_bpm: 92, new_bpm: null, from_ms: 41200, to_ms: 47100, hint: null }))
      .toBe('BPM 92.0 → 92.0, detected from 0:41.2 – 0:47.1');
  });
});

describe('detectRunning', () => {
  it('detects running status', () => {
    expect(detectRunning({ from_ms: 10500, to_ms: 17700, phase: 'checking' }))
      .toBe('Detecting the BPM from 0:10.5 – 0:17.7...');
    expect(detectRunning({ from_ms: 10500, to_ms: 17700, phase: 'waiting' }))
      .toBe('Detecting the BPM from 0:10.5 – 0:17.7 (waiting for the edge-AI server)...');
  });
});
