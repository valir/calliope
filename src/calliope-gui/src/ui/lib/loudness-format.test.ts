import { describe, expect, it } from 'vitest';
import { loudnessText } from './loudness-format';
import type { LoudnessOutcome, LoudnessResult } from './ipc';

const r = (outcome: LoudnessOutcome, lufs: number | null, gain_db: number, ceiling = -1): LoudnessResult => ({
  outcome, lufs, true_peak: -3, gain_db, target: -14, ceiling,
});

describe('loudnessText', () => {
  it('Save texts', () => {
    expect(loudnessText(r('normalized', -14, 3.2), 'save')).toBe('Saved, normalized to -14.0 LUFS (gain +3.2 dB).');
    expect(loudnessText(r('limited', -15.6, -1), 'save')).toBe(
      'Saved at -15.6 LUFS (gain -1.0 dB, limited by the -1 dBTP peak ceiling).');
    expect(loudnessText(r('too-quiet', null, 0), 'save')).toBe('Saved. Too quiet to normalize (below -70 LUFS).');
    expect(loudnessText(r('at-target', -14, 0), 'save')).toBe('Saved, normalized to -14.0 LUFS (gain +0.0 dB).');
    expect(loudnessText(r('at-ceiling', -15.6, 0), 'save')).toBe('Saved, normalized to -15.6 LUFS (gain +0.0 dB).');
  });

  it('Normalize texts', () => {
    expect(loudnessText(r('normalized', -14, 3.2), 'normalize')).toBe('Normalized to -14.0 LUFS (gain +3.2 dB).');
    expect(loudnessText(r('limited', -15.6, -1), 'normalize')).toBe(
      'Normalized to -15.6 LUFS (gain -1.0 dB, limited by the -1 dBTP peak ceiling).');
    expect(loudnessText(r('at-target', -14, 0), 'normalize')).toBe(
      'Already at the target (-14.0 LUFS); the file was not changed.');
    expect(loudnessText(r('at-ceiling', -15.6, 0), 'normalize')).toBe(
      'Already at the -1 dBTP peak ceiling (-15.6 LUFS); the file was not changed.');
    expect(loudnessText(r('too-quiet', null, 0), 'normalize')).toBe(
      'Too quiet to normalize (below -70 LUFS); the file was not changed.');
  });

  it('never shows -0.0', () => {
    expect(loudnessText(r('normalized', -14, -0.04), 'normalize')).toBe('Normalized to -14.0 LUFS (gain +0.0 dB).');
  });

  it('shows the ceiling without decimals only when whole', () => {
    expect(loudnessText(r('limited', -15.6, -1, -1), 'save')).toContain('the -1 dBTP');
    expect(loudnessText(r('limited', -15.6, -1, -1.5), 'save')).toContain('the -1.5 dBTP');
  });
});
