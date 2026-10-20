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
import { allGroupKeys, buildTree, filterTree, groupName, type BandNode } from './library-tree';
import { planRename, type RenamePlan } from './library-rename';

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
  /** A save found track.json changed on disk; the draft is kept until the user cancels. */
  conflict: false,
  message: '' as string,
  /** Inline error of the last action (shown as an alert). */
  errorMessage: '' as string,
  /** Validation errors next to the fields. */
  fieldErrors: {} as Partial<Record<DraftField, string>>,
  /** The tree item that has the focus (shared so that the Delete button can act on it). */
  treeFocus: null as string | null,
  /** A band / album rename waiting for the user's confirmation. */
  pendingRename: null as RenamePlan | null,
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
  lib.conflict = false;
  lib.message = '';
  lib.errorMessage = '';
  lib.fieldErrors = {};
  lib.treeFocus = null;
  lib.pendingRename = null;
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
  lib.pendingRename = null;
  lib.draft = null;
  lib.mode = 'view';
  lib.conflict = false;
  lib.fieldErrors = {};
  void frontendLog(`mode=view id=${id}`);
}

export const conflictText =
  'The track changed on disk. Cancel to reload it; your edits will be discarded.';

/** Leaves edit mode after a conflict (the user was told the edits are discarded) and reloads. */
export async function discardAfterConflict(): Promise<void> {
  endEdit();
  clearMessages();
  await load();
  lib.message = 'Reloaded the track from disk.';
}

/** The user said No to the rename confirmation: nothing is saved, the pane stays in edit mode. */
export function cancelRename(): void {
  if (!lib.pendingRename) return;
  lib.pendingRename = null;
  void frontendLog('rename cancelled');
}

export const groupConflictText =
  'Another track of this album or band changed on disk. Cancel to reload; your edits will be discarded.';

const tracksWord = (n: number): string => (n === 1 ? '1 track' : `${n} tracks`);

export async function saveEdit(confirmed = false): Promise<void> {
  const d = lib.draft;
  if (!d || lib.mode !== 'edit' || lib.busy) return;
  clearMessages();
  lib.pendingRename = null;
  const errors = validate(d);
  lib.fieldErrors = errors;
  if (Object.keys(errors).length > 0) {
    lib.errorMessage = 'Fix the highlighted fields, then save again.';
    return;
  }
  const edited = lib.library?.tracks.find((t) => t.id === d.id) ?? selectedTrack();
  const plan: RenamePlan = edited
    ? planRename(lib.library?.tracks ?? [], edited, d.band, d.album)
    : { kind: 'none' };
  if (plan.kind === 'refused') {
    lib.errorMessage = plan.message;
    void frontendLog(`rename refused reason=${plan.reason}`);
    return;
  }
  if (plan.kind === 'rename' && !confirmed) {
    lib.pendingRename = plan;
    return;
  }
  const request = toSaveRequest(d);
  if (plan.kind === 'join') {
    if (plan.position !== undefined) request.position = plan.position;
    if (plan.album_position !== undefined) request.album_position = plan.album_position;
  } else if (plan.kind === 'rename') {
    Object.assign(request, plan.main, { group: plan.group });
  }
  lib.busy = true;
  try {
    const res = await saveTrack(request);
    const changed = new Map([res.track, ...(res.updated ?? [])].map((t) => [t.id, t]));
    if (lib.library) lib.library.tracks = lib.library.tracks.map((t) => changed.get(t.id) ?? t);
    void frontendLog(
      `saved id=${res.track.id} tablatures=${res.track.tablatures.length} warnings=${res.warnings.length}`,
    );
    const failed = res.failed ?? [];
    if (plan.kind === 'rename') {
      void frontendLog(
        `rename kind=${plan.scope} from="${plan.from}" to="${plan.to}" tracks=${plan.count} merged=${plan.merged} failed=${failed.length}`,
      );
    }
    // Group keys are name-based: open the renamed track's band and album so it stays visible.
    const bandKey = `b:${groupName(res.track.band)}`;
    lib.expanded[bandKey] = true;
    lib.expanded[`${bandKey}|a:${groupName(res.track.album)}`] = true;
    endEdit();
    let message = res.warnings.length > 0 ? `Saved. ${res.warnings.join(' ')}` : 'Saved.';
    if (failed.length > 0) {
      message += ` Could not rename ${tracksWord(failed.length)}: ${failed.map((f) => `${f.title}: ${f.message}`).join('; ')}`;
      await load();
    }
    lib.message = message;
  } catch (err) {
    void frontendLog(`error save_track: ${String(err)}`);
    if (isConflict(err)) {
      // Keep edit mode and the draft: the user's typing is not thrown away.
      lib.conflict = true;
      const ownTitle = edited ? `"${edited.title}"` : null;
      const fromGroup = plan.kind === 'rename' && plan.group.length > 0 && !(ownTitle && String(err).includes(ownTitle));
      lib.errorMessage = fromGroup ? groupConflictText : conflictText;
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
