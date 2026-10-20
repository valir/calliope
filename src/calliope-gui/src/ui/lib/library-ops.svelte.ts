import {
  deleteTrack as ipcDeleteTrack,
  deleteTracks,
  frontendLog,
  updateTracks,
  type BatchFailure,
  type TrackRecord,
} from './ipc';
import { fullTree, lib, load, selectedTrack, visibleTree, isExpanded } from './library-state.svelte';
import { albumOrderPatches, gapToIndex, trackOrderPatches, type Patch } from './library-order';
import type { AlbumNode, BandNode } from './library-tree';

export type ReorderVia = 'key' | 'drag';

/** The reason a reorder is refused, else null. */
export function reorderAllowed(): 'search' | 'edit' | 'busy' | null {
  if (lib.search.trim() !== '') return 'search';
  if (lib.mode === 'edit') return 'edit';
  if (lib.busy) return 'busy';
  return null;
}

const tracksWord = (n: number): string => (n === 1 ? '1 track' : `${n} tracks`);
const isConflict = (err: unknown): boolean => /^(Error: )?conflict: /.test(String(err));
const failureList = (f: BatchFailure[]): string => f.map((x) => `"${x.title}": ${x.message}`).join('; ');

function none(key: string, via: ReorderVia, reason: string): void {
  void frontendLog(`reorder none key=${key} via=${via} reason=${reason}`);
}

type Found =
  | { kind: 'track'; band: BandNode; album: AlbumNode; index: number; count: number }
  | { kind: 'album'; band: BandNode; album: AlbumNode; index: number; count: number }
  | { kind: 'other'; reason: 'band' | 'no-album' | 'outside' };

function find(key: string): Found {
  for (const band of fullTree()) {
    if (band.key === key) return { kind: 'other', reason: 'band' };
    const real = band.albums.filter((a) => !a.isNone);
    for (const album of band.albums) {
      if (album.key === key) {
        if (album.isNone) return { kind: 'other', reason: 'no-album' };
        return { kind: 'album', band, album, index: real.indexOf(album), count: real.length };
      }
      const index = album.tracks.findIndex((t) => `t:${t.id}` === key);
      if (index >= 0) {
        if (album.isNone) return { kind: 'other', reason: 'no-album' };
        return { kind: 'track', band, album, index, count: album.tracks.length };
      }
    }
  }
  return { kind: 'other', reason: 'outside' };
}

async function reorder(found: Found & { kind: 'track' | 'album' }, to: number, via: ReorderVia): Promise<void> {
  const { band, album, index } = found;
  const isTrack = found.kind === 'track';
  const patches: Patch[] = isTrack ? trackOrderPatches(album, index, to) : albumOrderPatches(band, index, to);
  const what = isTrack
    ? `reorder tracks album=${album.key} id=${album.tracks[index].id}`
    : `reorder albums band=${band.key} album=${album.key}`;
  lib.message = '';
  lib.errorMessage = '';
  lib.busy = true;
  try {
    let written = 0;
    let failed: BatchFailure[] = [];
    if (patches.length > 0) {
      const res = await updateTracks(patches);
      const changed = new Map(res.tracks.map((t) => [t.id, t]));
      if (lib.library) lib.library.tracks = lib.library.tracks.map((t) => changed.get(t.id) ?? t);
      written = res.tracks.length;
      failed = res.failed;
    }
    void frontendLog(`${what} from=${index + 1} to=${to + 1} via=${via} written=${written} failed=${failed.length}`);
    if (failed.length > 0) {
      lib.errorMessage = `Could not save the new order for ${tracksWord(failed.length)}: ${failureList(failed)}`;
      await load();
    }
  } catch (err) {
    void frontendLog(`error update_tracks: ${String(err)}`);
    lib.errorMessage = isConflict(err)
      ? 'The album changed on disk and was reloaded. Try again.'
      : `Could not save the new order: ${String(err)}`;
    await load();
  } finally {
    lib.busy = false;
  }
}

/** Alt+Up / Alt+Down: moves the track (within its album) or the album (within its band) by one place. */
export async function moveTreeItem(key: string, delta: -1 | 1, via: 'key'): Promise<void> {
  const refused = reorderAllowed();
  if (refused) return none(key, via, refused);
  const found = find(key);
  if (found.kind === 'other') return none(key, via, found.reason);
  const to = found.index + delta;
  if (to < 0) return none(key, via, 'first');
  if (to >= found.count) return none(key, via, 'last');
  await reorder(found, to, via);
}

/** A pointer drop at `gap` (0 = before the first sibling) among the item's siblings. */
export async function dropTreeItem(key: string, gap: number, via: 'drag'): Promise<void> {
  const refused = reorderAllowed();
  if (refused) return none(key, via, refused);
  const found = find(key);
  if (found.kind === 'other') return none(key, via, found.reason);
  if (gap < 0 || gap > found.count) return none(key, via, 'outside');
  const to = gapToIndex(found.index, gap);
  if (to === null) return none(key, via, 'same');
  await reorder(found, to, via);
}

// ---- delete ----

export type DeleteTarget =
  | { kind: 'track'; key: string; track: TrackRecord }
  | { kind: 'album'; key: string; bandLabel: string; album: AlbumNode; tracks: TrackRecord[] };

export type DeleteNoneReason = 'band' | 'no-album' | 'none' | 'edit' | 'busy';

/** The target of Delete, or the reason there is none. */
export function deleteTargetOrReason(): { target: DeleteTarget; reason: null } | { target: null; reason: DeleteNoneReason; key: string } {
  const noneOf = (reason: DeleteNoneReason, key = '') => ({ target: null, reason, key });
  if (lib.mode === 'edit') return noneOf('edit', lib.treeFocus ?? '');
  if (lib.busy) return noneOf('busy', lib.treeFocus ?? '');
  const { tree, searching, expandKeys } = visibleTree();
  // Keys of the items the user can see (a group's children only while it is open).
  const visible = new Set<string>();
  for (const b of tree) {
    visible.add(b.key);
    if (!isExpanded(b.key, searching, expandKeys)) continue;
    for (const a of b.albums) {
      visible.add(a.key);
      if (!isExpanded(a.key, searching, expandKeys)) continue;
      for (const t of a.tracks) visible.add(`t:${t.id}`);
    }
  }
  let key = lib.treeFocus && visible.has(lib.treeFocus) ? lib.treeFocus : null;
  if (key === null) {
    const sel = selectedTrack();
    if (!sel) return noneOf('none');
    key = `t:${sel.id}`;
  }
  const found = find(key);
  if (found.kind === 'other') return noneOf(found.reason === 'outside' ? 'none' : found.reason, key);
  if (found.kind === 'track') return { target: { kind: 'track', key, track: found.album.tracks[found.index] }, reason: null };
  return {
    target: { kind: 'album', key, bandLabel: found.band.label, album: found.album, tracks: found.album.tracks },
    reason: null,
  };
}

export function treeDeleteTarget(): DeleteTarget | null {
  return deleteTargetOrReason().target;
}

/** Logs why a Delete key press did nothing. */
export function logDeleteNone(): void {
  const r = deleteTargetOrReason();
  if (r.target === null) void frontendLog(`delete none key=${r.key} reason=${r.reason}`);
}

/** Deletes the confirmed target. `removedKeys` are the tree keys that are gone afterwards. */
export async function deleteTreeTarget(target: DeleteTarget): Promise<{ removedKeys: Set<string> }> {
  const removedKeys = new Set<string>();
  if (lib.busy) return { removedKeys };
  lib.message = '';
  lib.errorMessage = '';
  lib.busy = true;
  try {
    if (target.kind === 'track') {
      const t = target.track;
      try {
        await ipcDeleteTrack(t.id, t.revision);
        void frontendLog(`deleted id=${t.id}`);
        removedKeys.add(target.key);
        if (lib.selectedId === t.id) lib.selectedId = null;
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
      }
      return { removedKeys };
    }
    const { album, tracks } = target;
    try {
      const res = await deleteTracks(tracks.map((t) => ({ id: t.id, revision: t.revision })));
      for (const id of res.deleted) {
        void frontendLog(`deleted id=${id}`);
        removedKeys.add(`t:${id}`);
        if (lib.selectedId === id) lib.selectedId = null;
      }
      if (res.failed.length === 0) removedKeys.add(target.key);
      void frontendLog(
        `deleted album band="${target.bandLabel}" album="${album.label}" tracks=${tracks.length} failed=${res.failed.length}`,
      );
      await load();
      if (res.failed.length > 0) {
        lib.errorMessage = `Deleted ${res.deleted.length} of ${tracks.length} tracks. Could not delete: ${failureList(res.failed)}`;
      } else {
        lib.message = `Deleted the album "${album.label}" (${tracksWord(tracks.length)}).`;
      }
    } catch (err) {
      void frontendLog(`error delete_tracks: ${String(err)}`);
      lib.errorMessage = `Could not delete: ${String(err)}`;
      await load();
    }
    return { removedKeys };
  } finally {
    lib.busy = false;
  }
}
