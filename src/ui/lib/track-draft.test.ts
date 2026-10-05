import { describe, it, expect } from 'vitest';
import {
  draftFromRecord, setField, isDirty, validate, toEdits, tabRows, stageAdd, stageUpdate,
  stageRemove, toSaveRequest, paneButtons, tabButtons, type Draft,
} from './track-draft';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { TrackRecord } from './ipc';

const slowBurn = FIXTURE_TRACKS[0];
const fresh = (r: TrackRecord = slowBurn): Draft => draftFromRecord(r);
const ok = (r: ReturnType<typeof stageAdd>): Draft => {
  if (!r.ok) throw new Error(r.error);
  return r.draft;
};
const pick = (token: string, name: string) => ({ token, name });

describe('draftFromRecord', () => {
  it('turns the record into form strings', () => {
    const d = fresh();
    expect(d.composers).toBe('Ann Example\nBo Sample');
    expect(d.year).toBe('2019');
    expect(d.source_url).toBe('https://example.org/slow-burn');
    expect(tabRows(d).map((r) => r.name)).toEqual(['slow-burn.gp5', 'slow-burn-solo.gp']);
  });
  it('null optionals become empty strings', () => {
    const d = fresh(FIXTURE_TRACKS[3]);
    expect([d.year, d.source_url, d.copyright, d.composers]).toEqual(['', '', '', 'Cy Placeholder']);
  });
  it('marks missing files', () => {
    const d = fresh({ ...slowBurn, missing: ['slow-burn-solo.gp'] });
    expect(tabRows(d).map((r) => r.state)).toEqual(['saved', 'missing']);
  });
});

describe('isDirty', () => {
  it('is false when nothing changed, true after an edit, false after reverting', () => {
    const d = fresh();
    expect(isDirty(d)).toBe(false);
    const e = setField(d, 'title', 'Other');
    expect(isDirty(e)).toBe(true);
    expect(isDirty(setField(e, 'title', 'Slow Burn'))).toBe(false);
  });
  it('is true after staging a tablature change', () => {
    expect(isDirty(ok(stageAdd(fresh(), pick('k', 'x.gp5'))))).toBe(true);
    expect(isDirty(ok(stageRemove(fresh(), 't0')))).toBe(true);
  });
  it('is false after add then remove again', () => {
    const d = ok(stageAdd(fresh(), pick('k', 'x.gp5')));
    expect(isDirty(ok(stageRemove(d, d.tabs[2].key)))).toBe(false);
  });
});

describe('validate', () => {
  const v = (patch: Partial<Record<string, string>>) => validate({ ...fresh(), ...patch });
  it('accepts the fixture records', () => {
    for (const r of FIXTURE_TRACKS) expect(validate(fresh(r))).toEqual({});
  });
  it('requires a title', () => {
    expect(v({ title: '   ' }).title).toMatch(/required/);
  });
  it('checks the year', () => {
    for (const y of ['0', '10000', '19.5', 'abc', '-3', '1e3']) expect(v({ year: y }).year).toBeTruthy();
    for (const y of ['', ' ', '1', '9999', ' 2020 ']) expect(v({ year: y }).year).toBeUndefined();
  });
  it('checks lengths (in characters) and control characters', () => {
    expect(v({ band: 'x'.repeat(201) }).band).toBeTruthy();
    expect(v({ band: 'é'.repeat(200) }).band).toBeUndefined();
    expect(v({ album: 'x'.repeat(201) }).album).toBeTruthy();
    expect(v({ title: 'a\u0007b' }).title).toBeTruthy();
    expect(v({ source_url: 'x'.repeat(2001) }).source_url).toBeTruthy();
    expect(v({ copyright: 'x'.repeat(501) }).copyright).toBeTruthy();
    expect(v({ copyright: 'x'.repeat(500) }).copyright).toBeUndefined();
  });
  it('limits composers', () => {
    const many = (n: number) => Array.from({ length: n }, (_, i) => `C${i}`).join('\n');
    expect(v({ composers: many(21) }).composers).toBeTruthy();
    expect(v({ composers: many(20) }).composers).toBeUndefined();
    expect(v({ composers: 'x'.repeat(201) }).composers).toBeTruthy();
  });
});

describe('toEdits', () => {
  it('trims, splits composers, and maps blanks to null', () => {
    const d = {
      ...fresh(), band: ' B ', title: ' T ', composers: ' A \r\n\n B \n', year: ' 2000 ',
      source_url: '  ', copyright: ' c ',
    };
    expect(toEdits(d)).toEqual({
      band: 'B', album: 'Northern Roads', title: 'T', composers: ['A', 'B'], year: 2000,
      source_url: null, copyright: 'c',
    });
    expect(toEdits({ ...d, year: '' }).year).toBeNull();
  });
});

describe('tablature staging', () => {
  it('add appends a "new" row', () => {
    const d = ok(stageAdd(fresh(), pick('k1', 'extra.gp5')));
    expect(tabRows(d).at(-1)).toMatchObject({ name: 'extra.gp5', state: 'new' });
  });
  it('rejects clashes case-insensitively, the audio name and track.json', () => {
    const d = fresh();
    for (const n of ['SLOW-BURN.GP5', 'Backing.MP3', 'track.json', 'Track.JSON']) {
      const r = stageAdd(d, pick('k', n));
      expect(r.ok).toBe(false);
    }
    expect(stageAdd(d, pick('k', 'SLOW-BURN.GP5'))).toEqual({
      ok: false,
      error: 'A tablature named "SLOW-BURN.GP5" is already listed',
    });
  });
  it('add, remove, add again', () => {
    let d = ok(stageAdd(fresh(), pick('k1', 'x.gp5')));
    const key = d.tabs[2].key;
    d = ok(stageRemove(d, key));
    expect(d.tabs).toHaveLength(2);
    d = ok(stageAdd(d, pick('k2', 'x.gp5')));
    expect(toSaveRequest(d).tablatures.at(-1)).toEqual({ kind: 'add', token: 'k2' });
    expect(new Set(d.tabs.map((t) => t.key)).size).toBe(3);
  });
  it('update marks an existing row "updated" with the picked name', () => {
    const d = ok(stageUpdate(fresh(), 't0', pick('k1', 'better.gp5')));
    expect(tabRows(d)[0]).toEqual({ key: 't0', name: 'better.gp5', state: 'updated' });
    expect(toSaveRequest(d).tablatures[0]).toEqual({
      kind: 'replace', name: 'slow-burn.gp5', token: 'k1',
    });
  });
  it('update with the same name is allowed', () => {
    const d = ok(stageUpdate(fresh(), 't0', pick('k1', 'SLOW-BURN.gp5')));
    expect(tabRows(d)[0].state).toBe('updated');
  });
  it('update rejects a name used by another row', () => {
    expect(stageUpdate(fresh(), 't0', pick('k', 'slow-burn-solo.gp')).ok).toBe(false);
  });
  it('update of a new row stays a new add with the new token', () => {
    let d = ok(stageAdd(fresh(), pick('k1', 'x.gp5')));
    d = ok(stageUpdate(d, d.tabs[2].key, pick('k2', 'y.gp5')));
    expect(tabRows(d)[2]).toMatchObject({ name: 'y.gp5', state: 'new' });
    expect(toSaveRequest(d).tablatures[2]).toEqual({ kind: 'add', token: 'k2' });
  });
  it('updating an updated row keeps the original name and takes the latest token', () => {
    let d = ok(stageUpdate(fresh(), 't0', pick('k1', 'a.gp5')));
    d = ok(stageUpdate(d, 't0', pick('k2', 'b.gp5')));
    expect(toSaveRequest(d).tablatures[0]).toEqual({
      kind: 'replace', name: 'slow-burn.gp5', token: 'k2',
    });
  });
  it('updating a missing row makes it "updated"', () => {
    const d0 = fresh({ ...slowBurn, missing: ['slow-burn.gp5'] });
    expect(tabRows(ok(stageUpdate(d0, 't0', pick('k', 'n.gp5'))))[0].state).toBe('updated');
  });
  it('removing an existing file then adding the same name becomes a replace', () => {
    let d = ok(stageRemove(fresh(), 't0'));
    d = ok(stageAdd(d, pick('k', 'slow-burn.gp5')));
    expect(toSaveRequest(d).tablatures).toEqual([
      { kind: 'keep', name: 'slow-burn-solo.gp' },
      { kind: 'replace', name: 'slow-burn.gp5', token: 'k' },
    ]);
  });
  it('unknown keys are errors', () => {
    expect(stageRemove(fresh(), 'nope').ok).toBe(false);
    expect(stageUpdate(fresh(), 'nope', pick('k', 'a.gp5')).ok).toBe(false);
  });
});

describe('toSaveRequest', () => {
  it('keeps unchanged files, omits removed ones, keeps the order', () => {
    let d = fresh(FIXTURE_TRACKS[4]);
    d = ok(stageRemove(d, 't1'));
    d = ok(stageUpdate(d, 't3', pick('u', 'intro2.gp')));
    d = ok(stageAdd(d, pick('a', 'new.gp5')));
    expect(toSaveRequest(setField(d, 'title', 'Open Water 2'))).toEqual({
      id: FIXTURE_TRACKS[4].id,
      revision: FIXTURE_TRACKS[4].revision,
      edits: { ...toEdits(d), title: 'Open Water 2' },
      tablatures: [
        { kind: 'keep', name: 'open-water-rhythm.gp5' },
        { kind: 'keep', name: 'open-water-bass.gp5' },
        { kind: 'replace', name: 'open-water-intro.gp', token: 'u' },
        { kind: 'keep', name: 'open-water-full.gpx' },
        { kind: 'add', token: 'a' },
      ],
    });
  });
});

describe('paneButtons', () => {
  const none = { edit: false, save: false, cancel: false, export: false, delete: false };
  it('no selection: everything disabled, in view or edit mode', () => {
    expect(paneButtons('view', false, false)).toEqual(none);
    expect(paneButtons('edit', false, false)).toEqual(none);
  });
  it('view mode with a selection', () => {
    expect(paneButtons('view', true, false)).toEqual({ ...none, edit: true, export: true, delete: true });
  });
  it('edit mode', () => {
    expect(paneButtons('edit', true, false)).toEqual({ ...none, save: true, cancel: true });
  });
  it('busy disables everything', () => {
    expect(paneButtons('view', true, true)).toEqual(none);
    expect(paneButtons('edit', true, true)).toEqual(none);
  });
});

describe('tabButtons', () => {
  const off = { add: false, update: false, export: false, remove: false };
  it('view mode: all disabled', () => {
    expect(tabButtons('view', null)).toEqual(off);
    expect(tabButtons('view', { state: 'saved' })).toEqual(off);
  });
  it('edit mode without a selection: only Add', () => {
    expect(tabButtons('edit', null)).toEqual({ ...off, add: true });
  });
  it('edit mode with a saved row: everything', () => {
    expect(tabButtons('edit', { state: 'saved' })).toEqual({ add: true, update: true, export: true, remove: true });
  });
  it('Export is disabled for new, updated and missing rows', () => {
    for (const state of ['new', 'updated', 'missing'] as const) {
      expect(tabButtons('edit', { state })).toEqual({ add: true, update: true, export: false, remove: true });
    }
  });
});
