process.env.TZ = 'Europe/Bucharest';
import { describe, expect, it } from 'vitest';
import {
  BUSY_TEXT, CHECKING_TEXT, SELECT_TRACK_TEXT, confirmText, countInText, disabledReason, exportRows, exportTarget, formatDay, formatStamp,
  lastText, progressText, resultText,
} from './export-format';
import type { ExportInfo, ExportSnapshot, TrackRecord } from './ipc';

const track = { id: 't1', title: 'T' } as TrackRecord;
const snap = (o: Partial<ExportSnapshot> = {}): ExportSnapshot => ({
  job: 'j', id: 't1', title: 'Stem Mix', target: 'mp3', path: '/m/Stem Mix.mp3', phase: 'writing',
  progress: 0.424, message: null, clipped: 0, info_error: null, ...o,
});
const info = (o: Partial<ExportInfo> = {}): ExportInfo => ({
  id: 't1', available: true, reason: null, source: 'backings/backing.flac',
  count_in: { bpm: 120, first_beat_ms: 0, lead_ms: 2500 }, memory: 1,
  last: { rc5: null, mp3: null, ogg: null }, ...o,
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
    expect(countInText(info())).toBe('Count-in: 5 beats at 120 BPM, the last one silent before the first beat (adds 2.5 s)');
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

describe('export info texts', () => {
  const none = { rc5: null, mp3: null, ogg: null };
  const last = {
    rc5: { memory: 7, at: '2026-10-24T18:30:00Z' },
    mp3: { path: '/m/A - B.mp3', at: '2026-10-24T18:32:00Z' },
    ogg: null,
  };
  it('formats local time and rejects bad values', () => {
    expect(formatStamp('2026-10-24T18:30:00Z')).toBe('2026-10-24 21:30');
    expect(formatDay('2026-10-24T22:30:00Z')).toBe('2026-10-25');
    for (const bad of ['', 'nonsense']) {
      expect(formatStamp(bad)).toBe('');
      expect(formatDay(bad)).toBe('');
    }
  });
  it('exportRows lists the present kinds in order', () => {
    expect(exportRows(last)).toEqual([
      { id: 'track-export-rc5', label: 'RC-5', value: 'memory 7 (2026-10-24 21:30)' },
      { id: 'track-export-mp3', label: 'MP3', value: '/m/A - B.mp3 (2026-10-24 21:32)' },
    ]);
    expect(exportRows(none)).toEqual([]);
    expect(exportRows(undefined)).toEqual([]);
    expect(exportRows({ ...none, ogg: { path: '/o.ogg', at: '' } })[0].value).toBe('/o.ogg');
  });
  it('lastText', () => {
    expect(lastText('rc5', last)).toBe('Last: memory 7, 2026-10-24');
    expect(lastText('mp3', last)).toBe('Last: /m/A - B.mp3, 2026-10-24');
    expect(lastText('ogg', last)).toBe('');
    expect(lastText('ogg', undefined)).toBe('');
  });
});
