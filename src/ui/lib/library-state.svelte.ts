import {
  deleteTrack as ipcDeleteTrack,
  exportTrack,
  frontendLog,
  getRepository,
  listTracks,
  saveTrack,
  type Library,
  type RepoStatus,
  type TrackRecord,
} from './ipc';
import { draftFromRecord, toSaveRequest, validate, type Draft, type DraftField, type Mode } from './track-draft';
import { allGroupKeys, buildTree, filterTree, type BandNode } from './library-tree';

export const lib = $state({
  library: null as Library | null,
  loaded: false,
  loading: false,
  /** Load failure, shown as an alert. */
  error: '',
  /** Set when the repository folder is missing or written by a newer Calliope. */
  repoProblem: '' as string,
  expanded: {} as Record<string, boolean>,
  /** Groups the user closed while a search was showing everything expanded. */
  searchCollapsed: {} as Record<string, boolean>,
  search: '',
  selectedId: null as string | null,
  mode: 'view' as Mode,
  draft: null as Draft | null,
  busy: false,
  message: '' as string,
  /** Inline error of the last action (shown as an alert). */
  errorMessage: '' as string,
  /** Validation errors next to the fields. */
  fieldErrors: {} as Partial<Record<DraftField, string>>,
});

export function resetLibrary(): void {
  lib.library = null;
  lib.loaded = false;
  lib.loading = false;
  lib.error = '';
  lib.repoProblem = '';
  lib.expanded = {};
  lib.searchCollapsed = {};
  lib.search = '';
  lib.selectedId = null;
  lib.mode = 'view';
  lib.draft = null;
  lib.busy = false;
  lib.message = '';
  lib.errorMessage = '';
  lib.fieldErrors = {};
}

function repoProblemText(status: RepoStatus, root: string): string {
  if (status === 'missing') return `The track repository folder "${root}" does not exist. Choose another one in Settings.`;
  return `The track repository at "${root}" was written by a newer Calliope. Choose another folder in Settings.`;
}

export async function load(): Promise<void> {
  lib.loading = true;
  lib.error = '';
  try {
    let info = await getRepository();
    // A missing default root is created by list_tracks (first start just works);
    // a configured root that does not exist is never created.
    if ((info.status === 'missing' && !info.is_default) || info.status === 'newer') {
      lib.library = { root: info.root, tracks: [], problems: [] };
      lib.repoProblem = repoProblemText(info.status, info.root);
      void frontendLog(`library root=${info.root} status=${info.status} tracks=0 problems=0`);
    } else {
      lib.repoProblem = '';
      const l = await listTracks();
      if (info.status === 'missing') info = await getRepository();
      lib.library = l;
      void frontendLog(
        `library root=${l.root} status=${info.status} tracks=${l.tracks.length} problems=${l.problems.length}`,
      );
      if (lib.selectedId && !l.tracks.some((t) => t.id === lib.selectedId)) lib.selectedId = null;
    }
  } catch (err) {
    lib.error = `Could not read the track library: ${String(err)}`;
    void frontendLog(`error list_tracks: ${String(err)}`);
  } finally {
    lib.loading = false;
    lib.loaded = true;
  }
}

export function fullTree(): BandNode[] {
  return buildTree(lib.library?.tracks ?? []);
}

export function visibleTree(): { tree: BandNode[]; searching: boolean; expandKeys: Set<string> } {
  const searching = lib.search.trim() !== '';
  const { tree, expandKeys } = filterTree(fullTree(), lib.search);
  return { tree, searching, expandKeys: new Set(expandKeys) };
}

export function isExpanded(key: string, searching: boolean, expandKeys: Set<string>): boolean {
  return searching ? expandKeys.has(key) && !lib.searchCollapsed[key] : !!lib.expanded[key];
}

export function toggleGroup(key: string, searching: boolean, expandKeys: Set<string>): void {
  const open = isExpanded(key, searching, expandKeys);
  if (searching) lib.searchCollapsed[key] = open;
  else lib.expanded[key] = !open;
}

export function setSearch(q: string): void {
  lib.search = q;
  lib.searchCollapsed = {};
}

export function expandAll(): void {
  const next: Record<string, boolean> = {};
  for (const k of allGroupKeys(fullTree())) next[k] = true;
  lib.expanded = next;
  lib.searchCollapsed = {};
}

export function collapseAll(): void {
  lib.expanded = {};
  if (lib.search.trim() !== '') {
    const { expandKeys } = filterTree(fullTree(), lib.search);
    lib.searchCollapsed = Object.fromEntries(expandKeys.map((k) => [k, true]));
  }
}

export function selectTrack(id: string): void {
  if (lib.selectedId === id) return;
  lib.selectedId = id;
  lib.message = '';
  lib.errorMessage = '';
  void frontendLog(`select id=${id}`);
}

export function selectedTrack(): TrackRecord | null {
  return lib.library?.tracks.find((t) => t.id === lib.selectedId) ?? null;
}

const isConflict = (err: unknown): boolean => /^(Error: )?conflict: /.test(String(err));

function clearMessages(): void {
  lib.message = '';
  lib.errorMessage = '';
}

export function startEdit(): void {
  const t = selectedTrack();
  if (!t || lib.mode === 'edit' || lib.busy) return;
  clearMessages();
  lib.fieldErrors = {};
  lib.draft = draftFromRecord(t);
  lib.mode = 'edit';
  void frontendLog(`mode=edit id=${t.id}`);
}

export function endEdit(): void {
  const id = lib.selectedId;
  lib.draft = null;
  lib.mode = 'view';
  lib.fieldErrors = {};
  void frontendLog(`mode=view id=${id}`);
}

const conflictText =
  'This track was changed on disk since it was loaded. Your edits were not saved and the track was reloaded.';

export async function saveEdit(): Promise<void> {
  const d = lib.draft;
  if (!d || lib.mode !== 'edit' || lib.busy) return;
  clearMessages();
  const errors = validate(d);
  lib.fieldErrors = errors;
  if (Object.keys(errors).length > 0) {
    lib.errorMessage = 'Fix the highlighted fields, then save again.';
    return;
  }
  lib.busy = true;
  try {
    const res = await saveTrack(toSaveRequest(d));
    const tracks = lib.library?.tracks;
    if (lib.library && tracks) {
      lib.library.tracks = tracks.map((t) => (t.id === res.track.id ? res.track : t));
    }
    void frontendLog(
      `saved id=${res.track.id} tablatures=${res.track.tablatures.length} warnings=${res.warnings.length}`,
    );
    endEdit();
    lib.message = res.warnings.length > 0 ? `Saved. ${res.warnings.join(' ')}` : 'Saved.';
  } catch (err) {
    void frontendLog(`error save_track: ${String(err)}`);
    if (isConflict(err)) {
      endEdit();
      lib.errorMessage = conflictText;
      await load();
    } else {
      lib.errorMessage = `Could not save: ${String(err)}`;
    }
  } finally {
    lib.busy = false;
  }
}

export async function removeSelectedTrack(): Promise<void> {
  const t = selectedTrack();
  if (!t || lib.mode !== 'view' || lib.busy) return;
  clearMessages();
  lib.busy = true;
  try {
    await ipcDeleteTrack(t.id, t.revision);
    void frontendLog(`deleted id=${t.id}`);
    lib.selectedId = null;
    await load();
    lib.message = `Deleted "${t.title}".`;
  } catch (err) {
    void frontendLog(`error delete_track: ${String(err)}`);
    if (isConflict(err)) {
      lib.errorMessage = 'This track was changed on disk since it was loaded, so it was not deleted. The track was reloaded.';
      await load();
    } else {
      lib.errorMessage = `Could not delete: ${String(err)}`;
    }
  } finally {
    lib.busy = false;
  }
}

export async function exportSelectedTrack(): Promise<void> {
  const t = selectedTrack();
  if (!t || lib.mode !== 'view' || lib.busy) return;
  clearMessages();
  lib.busy = true;
  try {
    const path = await exportTrack(t.id);
    if (path) {
      lib.message = `Exported to ${path}`;
      void frontendLog(`exported id=${t.id}`);
    }
  } catch (err) {
    void frontendLog(`error export_track: ${String(err)}`);
    lib.errorMessage = `Could not export: ${String(err)}`;
  } finally {
    lib.busy = false;
  }
}
