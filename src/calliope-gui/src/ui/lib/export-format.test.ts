import { describe, expect, it } from 'vitest';
import {
  BUSY_TEXT, CHECKING_TEXT, SELECT_TRACK_TEXT, confirmText, countInText, disabledReason, exportTarget,
  progressText, resultText,
} from './export-format';
import type { ExportInfo, ExportSnapshot, TrackRecord } from './ipc';

const track = { id: 't1', title: 'T' } as TrackRecord;
const snap = (o: Partial<ExportSnapshot> = {}): ExportSnapshot => ({
  job: 'j', id: 't1', title: 'Stem Mix', target: 'mp3', path: '/m/Stem Mix.mp3', phase: 'writing',
  progress: 0.424, message: null, clipped: 0, ...o,
});
const info = (o: Partial<ExportInfo> = {}): ExportInfo => ({
  id: 't1', available: true, reason: null, source: 'backings/backing.flac',
  count_in: { bpm: 120, first_beat_ms: 0, lead_ms: 2000 }, memory: 1, ...o,
});

describe('exportTarget', () => {
  it('band and album keys are groups, even with a selected track', () => {
    expect(exportTarget('b:x', track)).toEqual({ kind: 'group' });
    expect(exportTarget('b:x|a:y', track)).toEqual({ kind: 'group' });
  });
  it('a track key or no focus uses the selected track', () => {
    expect(exportTarget('t:t1', track)).toEqual({ kind: 'track', track });
    expect(exportTarget(null, track)).toEqual({ kind: 'track', track });
    expect(exportTarget(null, null)).toEqual({ kind: 'none' });
    expect(exportTarget('t:t1', null)).toEqual({ kind: 'none' });
  });
});

describe('texts', () => {
  it('progressText', () => {
    expect(progressText(snap())).toBe('Exporting Stem Mix to /m/Stem Mix.mp3 · 42 %');
  });
  it('countInText', () => {
    expect(countInText(info())).toBe('Count-in: 4 beats at 120 BPM, ending on the first beat (adds 2.0 s)');
    expect(countInText(info({ count_in: null }))).toBe('');
    expect(countInText(null)).toBe('');
  });
  it('resultText', () => {
    expect(resultText(snap({ phase: 'done' }))).toBe('Written to /m/Stem Mix.mp3.');
    expect(resultText(snap({ phase: 'done', target: 'rc5', path: '/r/ROLAND/WAVE/007_1/007_1.wav' })))
      .toBe('Written to /r/ROLAND/WAVE/007_1/007_1.wav. Eject the RC-5 before unplugging it.');
    expect(resultText(snap({ phase: 'failed', message: 'disk full' }))).toBe('Export failed: disk full');
    expect(resultText(snap({ phase: 'cancelled' }))).toBe('Export cancelled; nothing was written.');
    expect(resultText(snap())).toBe('');
    expect(resultText(null)).toBe('');
  });
  it('confirmText names the file', () => {
    expect(confirmText('/m/a.mp3')).toBe('Replace the existing file /m/a.mp3? Its old content will be lost.');
  });
});

describe('disabledReason', () => {
  const t = { kind: 'track', track } as const;
  it('enabled for an available track', () => expect(disabledReason(t, info(), false)).toBeNull());
  it('busy wins', () => expect(disabledReason(t, info(), true)).toBe(BUSY_TEXT));
  it('group or none', () => {
    expect(disabledReason({ kind: 'group' }, null, false)).toBe(SELECT_TRACK_TEXT);
    expect(disabledReason({ kind: 'none' }, null, false)).toBe(SELECT_TRACK_TEXT);
  });
  it('the info reason, and waiting for the info', () => {
    expect(disabledReason(t, info({ available: false, reason: 'No mix saved yet: save one in the Editor' }), false))
      .toBe('No mix saved yet: save one in the Editor');
    expect(disabledReason(t, null, false)).toBe(CHECKING_TEXT);
    expect(disabledReason(t, info({ id: 'other' }), false)).toBe(CHECKING_TEXT);
  });
});
