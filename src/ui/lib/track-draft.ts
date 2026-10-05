import type { SaveTrackRequest, TabEntry, TrackEdits, TrackRecord } from './ipc';

export type Mode = 'view' | 'edit';
export type TabState = 'saved' | 'new' | 'updated' | 'missing';
export type DraftField =
  | 'band' | 'album' | 'title' | 'composers' | 'year' | 'source_url' | 'copyright';

export interface DraftFields {
  band: string;
  album: string;
  title: string;
  /** One composer per line. */
  composers: string;
  /** Blank = none. */
  year: string;
  source_url: string;
  copyright: string;
}

/** A tablature row. `orig` is the name in the repository (null for a staged add). */
export interface DraftTab {
  key: string;
  name: string;
  orig: string | null;
  /** Pick token for a staged add / replace. */
  token: string | null;
  missing: boolean;
}

export interface Draft extends DraftFields {
  id: string;
  revision: string;
  audio: string;
  tabs: DraftTab[];
  /** Everything as loaded, to detect changes. */
  initial: { fields: DraftFields; tabs: DraftTab[] };
  nextKey: number;
}

export type Result = { ok: true; draft: Draft } | { ok: false; error: string };

const MAX_NAME = 200;
const MAX_COMPOSERS = 20;
const MAX_URL = 2000;
const MAX_COPYRIGHT = 500;
const FIELD_KEYS: DraftField[] = [
  'band', 'album', 'title', 'composers', 'year', 'source_url', 'copyright',
];

export function draftFromRecord(r: TrackRecord): Draft {
  const fields: DraftFields = {
    band: r.band,
    album: r.album,
    title: r.title,
    composers: r.composers.join('\n'),
    year: r.year === null ? '' : String(r.year),
    source_url: r.source_url ?? '',
    copyright: r.copyright ?? '',
  };
  const tabs: DraftTab[] = r.tablatures.map((name, i) => ({
    key: `t${i}`,
    name,
    orig: name,
    token: null,
    missing: r.missing.includes(name),
  }));
  return {
    ...fields,
    id: r.id,
    revision: r.revision,
    audio: r.audio,
    tabs,
    initial: { fields, tabs },
    nextKey: tabs.length,
  };
}

const fieldsOf = (d: DraftFields): DraftFields =>
  Object.fromEntries(FIELD_KEYS.map((k) => [k, d[k]])) as unknown as DraftFields;

export function setField(d: Draft, field: DraftField, value: string): Draft {
  return { ...d, [field]: value };
}

export function tabState(t: DraftTab): TabState {
  if (t.orig === null) return 'new';
  if (t.token !== null) return 'updated';
  return t.missing ? 'missing' : 'saved';
}

const sameTabs = (a: DraftTab[], b: DraftTab[]): boolean =>
  a.length === b.length &&
  a.every((t, i) => t.key === b[i].key && t.name === b[i].name && t.token === b[i].token);

/** True when the form or the tablature list differs from the loaded record. */
export function isDirty(d: Draft): boolean {
  const cur = fieldsOf(d);
  return (
    FIELD_KEYS.some((k) => cur[k] !== d.initial.fields[k]) || !sameTabs(d.tabs, d.initial.tabs)
  );
}

// ---- validation (mirrors src/track_meta.rs `validate_edit_fields`) ----

const hasControl = (s: string): boolean => /\p{Cc}/u.test(s);
const len = (s: string): number => [...s].length;

function textError(label: string, s: string, max: number): string | null {
  if (len(s) > max) return `${label} must be at most ${max} characters`;
  if (hasControl(s)) return `${label} must not contain control characters`;
  return null;
}

export function composerList(composers: string): string[] {
  const lines = composers.split(/\r?\n/).map((c) => c.trim());
  // Blank lines (typically a trailing one) are ignored, like empty form rows.
  return lines.filter((c) => c !== '');
}

export function validate(d: DraftFields): Partial<Record<DraftField, string>> {
  const errors: Partial<Record<DraftField, string>> = {};
  const set = (f: DraftField, m: string | null) => {
    if (m && !errors[f]) errors[f] = m;
  };
  set('band', textError('Band', d.band.trim(), MAX_NAME));
  set('album', textError('Album', d.album.trim(), MAX_NAME));
  if (d.title.trim() === '') set('title', 'Title is required');
  set('title', textError('Title', d.title.trim(), MAX_NAME));
  const composers = composerList(d.composers);
  if (composers.length > MAX_COMPOSERS) set('composers', `At most ${MAX_COMPOSERS} composers`);
  for (const c of composers) set('composers', textError('Composer', c, MAX_NAME));
  const year = d.year.trim();
  if (year !== '') {
    if (!/^\d+$/.test(year) || Number(year) < 1 || Number(year) > 9999) {
      set('year', 'Year must be a whole number from 1 to 9999');
    }
  }
  set('source_url', textError('Source link', d.source_url.trim(), MAX_URL));
  set('copyright', textError('Copyright', d.copyright.trim(), MAX_COPYRIGHT));
  return errors;
}

/** The edits sent to Rust (values trimmed here too; Rust trims again). Call after `validate`. */
export function toEdits(d: DraftFields): TrackEdits {
  const opt = (s: string): string | null => (s.trim() === '' ? null : s.trim());
  const year = d.year.trim();
  return {
    band: d.band.trim(),
    album: d.album.trim(),
    title: d.title.trim(),
    composers: composerList(d.composers),
    year: year === '' ? null : Number(year),
    source_url: opt(d.source_url),
    copyright: opt(d.copyright),
  };
}

// ---- tablature staging ----

export function tabRows(d: Draft): { key: string; name: string; state: TabState }[] {
  return d.tabs.map((t) => ({ key: t.key, name: t.name, state: tabState(t) }));
}

/** Names that a new tablature must not take: other rows (case-insensitive), the audio, track.json. */
function clash(d: Draft, name: string, exceptKey: string | null): string | null {
  const lower = name.toLowerCase();
  if (lower === 'track.json') return '"track.json" is reserved';
  if (lower === d.audio.toLowerCase()) return `"${name}" is the audio file of this track`;
  if (d.tabs.some((t) => t.key !== exceptKey && t.name.toLowerCase() === lower)) {
    return `A tablature named "${name}" is already listed`;
  }
  return null;
}

/**
 * Appends a picked file as a new row. If the name is exactly that of a tablature removed
 * earlier in this edit, it becomes a replace of that file (so Save does not clobber it).
 */
export function stageAdd(d: Draft, picked: { token: string; name: string }): Result {
  const error = clash(d, picked.name, null);
  if (error) return { ok: false, error };
  const removed = d.initial.tabs.find(
    (t) => t.name === picked.name && !d.tabs.some((c) => c.key === t.key),
  );
  const row: DraftTab = removed
    ? { ...removed, token: picked.token }
    : { key: `t${d.nextKey}`, name: picked.name, orig: null, token: picked.token, missing: false };
  return {
    ok: true,
    draft: { ...d, tabs: [...d.tabs, row], nextKey: removed ? d.nextKey : d.nextKey + 1 },
  };
}

/** Replaces the row's file. A "new" row stays a new add (with the new token). */
export function stageUpdate(d: Draft, key: string, picked: { token: string; name: string }): Result {
  const i = d.tabs.findIndex((t) => t.key === key);
  if (i < 0) return { ok: false, error: 'No such tablature' };
  const error = clash(d, picked.name, key);
  if (error) return { ok: false, error };
  const old = d.tabs[i];
  const row: DraftTab = { ...old, name: picked.name, token: picked.token, missing: false };
  const tabs = d.tabs.map((t, j) => (j === i ? row : t));
  return { ok: true, draft: { ...d, tabs } };
}

export function stageRemove(d: Draft, key: string): Result {
  if (!d.tabs.some((t) => t.key === key)) return { ok: false, error: 'No such tablature' };
  return { ok: true, draft: { ...d, tabs: d.tabs.filter((t) => t.key !== key) } };
}

/** Existing names that are left out are removed by Rust; list order is the new order. */
export function toSaveRequest(d: Draft): SaveTrackRequest {
  const tablatures: TabEntry[] = d.tabs.map((t): TabEntry => {
    if (t.orig === null) return { kind: 'add', token: t.token as string };
    if (t.token !== null) return { kind: 'replace', name: t.orig, token: t.token };
    return { kind: 'keep', name: t.orig };
  });
  return { id: d.id, revision: d.revision, edits: toEdits(d), tablatures };
}

// ---- button states (plan §2.8, AC8, AC9, AC12, AC13) ----

export interface PaneButtons { edit: boolean; save: boolean; cancel: boolean; export: boolean; delete: boolean }
export interface TabButtons { add: boolean; update: boolean; export: boolean; remove: boolean }

export function paneButtons(mode: Mode, hasSelection: boolean, busy: boolean): PaneButtons {
  const off = { edit: false, save: false, cancel: false, export: false, delete: false };
  if (busy || !hasSelection) return off;
  if (mode === 'edit') return { ...off, save: true, cancel: true };
  return { ...off, edit: true, export: true, delete: true };
}

export function tabButtons(mode: Mode, selectedRow: { state: TabState } | null): TabButtons {
  if (mode !== 'edit') return { add: false, update: false, export: false, remove: false };
  return {
    add: true,
    update: selectedRow !== null,
    remove: selectedRow !== null,
    export: selectedRow !== null && selectedRow.state === 'saved',
  };
}
