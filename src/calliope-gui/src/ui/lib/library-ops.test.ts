import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { TrackPatch, TrackRecord } from './ipc';
import { lib, load, resetLibrary, saveEdit, startEdit, selectTrack } from './library-state.svelte';
import {
  deleteTargetOrReason, deleteTreeTarget, dropTreeItem, moveTreeItem, treeDeleteTarget,
} from './library-ops.svelte';

const rec = (id: string, band: string, album: string, title: string, position: number | null = null,
  album_position: number | null = null): TrackRecord => ({
  ...FIXTURE_TRACKS[0], id, band, album, title, position, album_position, revision: `r-${id}`,
});

let disk: TrackRecord[] = [];
let calls: [string, unknown][] = [];
let logs: string[] = [];
let failIds = new Set<string>();

beforeEach(() => {
  disk = [
    rec('a', 'Band', 'Order', 'A'), rec('b', 'Band', 'Order', 'B'), rec('c', 'Band', 'Order', 'C'),
    rec('n', 'Band', '', 'Loose'),
  ];
  calls = [];
  logs = [];
  failIds = new Set();
  resetLibrary();
  mockIPC((cmd, args) => {
    const a = args as Record<string, any>;
    if (cmd === 'frontend_log') { logs.push(a.message); return null; }
    if (cmd === 'get_repository') return { root: '/r', is_default: true, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: disk, problems: [] };
    calls.push([cmd, args]);
    if (cmd === 'update_tracks') {
      const failed: unknown[] = [];
      const tracks: TrackRecord[] = [];
      for (const p of a.patches as TrackPatch[]) {
        const i = disk.findIndex((t) => t.id === p.id);
        if (failIds.has(p.id)) { failed.push({ id: p.id, title: disk[i].title, message: 'disk full' }); continue; }
        const { id: _i, revision: _r, ...rest } = p;
        disk[i] = { ...disk[i], ...rest, revision: `${disk[i].revision}+` };
        tracks.push(disk[i]);
      }
      return { tracks, failed };
    }
    if (cmd === 'delete_tracks') {
      const deleted: string[] = [];
      const failed: unknown[] = [];
      for (const it of a.items as { id: string }[]) {
        const t = disk.find((x) => x.id === it.id)!;
        if (failIds.has(it.id)) failed.push({ id: it.id, title: t.title, message: 'conflict: changed on disk' });
        else { deleted.push(it.id); disk = disk.filter((x) => x.id !== it.id); }
      }
      return { deleted, failed };
    }
    if (cmd === 'save_track') {
      const r = a.request;
      const cur = disk.find((t) => t.id === r.id)!;
      const track = { ...cur, ...r.edits, position: r.position ?? cur.position, album_position: r.album_position ?? cur.album_position };
      const updated: TrackRecord[] = [];
      for (const p of (r.group ?? []) as TrackPatch[]) {
        const i = disk.findIndex((t) => t.id === p.id);
        const { id: _i, revision: _r, ...rest } = p;
        disk[i] = { ...disk[i], ...rest };
        updated.push(disk[i]);
      }
      disk = disk.map((t) => (t.id === track.id ? track : t));
      return { track, warnings: [], updated, failed: [] };
    }
    return undefined;
  });
});
afterEach(() => clearMocks());

const album = 'b:band|a:order';
const patchesOf = (cmd: string) => calls.filter(([c]) => c === cmd).map(([, a]) => a as any);
const order = () =>
  lib.library!.tracks.filter((t) => t.album === 'Order').sort((x, y) => (x.position ?? 99) - (y.position ?? 99)).map((t) => t.id);

describe('moveTreeItem / dropTreeItem', () => {
  it('Alt+Up sends the patches and updates the tracks', async () => {
    await load();
    await moveTreeItem('t:c', -1, 'key');
    expect(patchesOf('update_tracks')[0].patches).toEqual([
      { id: 'a', revision: 'r-a', position: 1 },
      { id: 'c', revision: 'r-c', position: 2 },
      { id: 'b', revision: 'r-b', position: 3 },
    ]);
    expect(order()).toEqual(['a', 'c', 'b']);
    expect(logs).toContain(`reorder tracks album=${album} id=c from=3 to=2 via=key written=3 failed=0`);
    expect(lib.busy).toBe(false);
  });
  it('a drop moves by gap, and a gap that changes nothing is logged', async () => {
    await load();
    await dropTreeItem('t:c', 0, 'drag');
    expect(order()).toEqual(['c', 'a', 'b']);
    await dropTreeItem('t:c', 1, 'drag');
    expect(logs).toContain('reorder none key=t:c via=drag reason=same');
    await dropTreeItem('t:c', 9, 'drag');
    expect(logs).toContain('reorder none key=t:c via=drag reason=outside');
    expect(patchesOf('update_tracks')).toHaveLength(1);
  });
  it('moves albums within the band', async () => {
    disk.push(rec('g', 'Band', 'Zeta', 'G'));
    await load();
    await moveTreeItem('b:band|a:zeta', -1, 'key');
    const p = patchesOf('update_tracks')[0].patches as TrackPatch[];
    expect(p.every((x) => x.album_position !== undefined && x.position === undefined)).toBe(true);
    expect(p.find((x) => x.id === 'g')!.album_position).toBe(1);
    expect(logs.some((l) => l.startsWith(`reorder albums band=b:band album=b:band|a:zeta from=2 to=1 via=key written=4`))).toBe(true);
  });
  it('sends nothing for the first item, a band, "(no album)", search, edit and busy', async () => {
    await load();
    await moveTreeItem('t:a', -1, 'key');
    await moveTreeItem('t:c', 1, 'key');
    await moveTreeItem('b:band', 1, 'key');
    await moveTreeItem('b:band|a:', 1, 'key');
    await moveTreeItem('t:n', 1, 'key');
    lib.search = 'a';
    await moveTreeItem('t:b', 1, 'key');
    lib.search = '';
    lib.mode = 'edit';
    await moveTreeItem('t:b', 1, 'key');
    lib.mode = 'view';
    lib.busy = true;
    await moveTreeItem('t:b', 1, 'key');
    lib.busy = false;
    expect(calls).toEqual([]);
    expect(logs.filter((l) => l.startsWith('reorder none')).map((l) => l.replace('reorder none ', ''))).toEqual([
      'key=t:a via=key reason=first',
      'key=t:c via=key reason=last',
      'key=b:band via=key reason=band',
      'key=b:band|a: via=key reason=no-album',
      'key=t:n via=key reason=no-album',
      'key=t:b via=key reason=search',
      'key=t:b via=key reason=edit',
      'key=t:b via=key reason=busy',
    ]);
  });
  it('a partial failure sets the error message and reloads', async () => {
    await load();
    failIds.add('a');
    await moveTreeItem('t:c', -1, 'key');
    expect(lib.errorMessage).toBe('Could not save the new order for 1 track: "A": disk full');
    // reloaded from "disk": the written tracks are there, A is unchanged
    expect(lib.library!.tracks.find((t) => t.id === 'c')!.position).toBe(2);
    expect(lib.library!.tracks.find((t) => t.id === 'a')!.position).toBeNull();
  });
  it('a conflict reloads with a retry message', async () => {
    await load();
    clearMocks();
    mockIPC((cmd) => {
      if (cmd === 'update_tracks') throw new Error('conflict: "A" changed on disk since it was loaded');
      if (cmd === 'get_repository') return { root: '/r', is_default: true, status: 'ok' };
      if (cmd === 'list_tracks') return { root: '/r', tracks: disk, problems: [] };
      return null;
    });
    await moveTreeItem('t:c', -1, 'key');
    expect(lib.errorMessage).toBe('The album changed on disk and was reloaded. Try again.');
  });
});

describe('delete', () => {
  it('finds the focused track, the focused album, or the selected track; refuses a band', async () => {
    await load();
    expect(treeDeleteTarget()).toBeNull();
    selectTrack('b');
    expect(treeDeleteTarget()).toMatchObject({ kind: 'track', key: 't:b' });
    lib.expanded = { 'b:band': true, [album]: true };
    lib.treeFocus = album;
    const t = treeDeleteTarget();
    expect(t).toMatchObject({ kind: 'album', bandLabel: 'Band' });
    expect(t?.kind === 'album' && t.tracks.length).toBe(3);
    lib.treeFocus = 'b:band';
    expect(deleteTargetOrReason()).toMatchObject({ reason: 'band' });
    lib.treeFocus = 'b:band|a:';
    expect(deleteTargetOrReason()).toMatchObject({ reason: 'no-album' });
    lib.mode = 'edit';
    expect(deleteTargetOrReason()).toMatchObject({ reason: 'edit' });
  });
  it('an album delete sends every track of the full album even while searching, and reports failures', async () => {
    await load();
    lib.expanded = { 'b:band': true, [album]: true };
    lib.search = 'B';
    lib.treeFocus = album;
    const target = treeDeleteTarget()!;
    failIds.add('b');
    const { removedKeys } = await deleteTreeTarget(target);
    expect(patchesOf('delete_tracks')[0].items.map((i: any) => i.id)).toEqual(['a', 'b', 'c']);
    expect(lib.errorMessage).toBe('Deleted 2 of 3 tracks. Could not delete: "B": conflict: changed on disk');
    expect([...removedKeys].sort()).toEqual(['t:a', 't:c']);
    expect(logs).toContain('deleted album band="Band" album="Order" tracks=3 failed=1');
    expect(lib.library!.tracks.map((t) => t.id)).toEqual(['b', 'n']);
  });
  it('a whole album delete reports success and clears the selection', async () => {
    await load();
    selectTrack('a');
    lib.treeFocus = album;
    lib.expanded = { 'b:band': true };
    const { removedKeys } = await deleteTreeTarget(treeDeleteTarget()!);
    expect(removedKeys.has(album)).toBe(true);
    expect(lib.message).toBe('Deleted the album "Order" (3 tracks).');
    expect(lib.selectedId).toBeNull();
  });
});

describe('saveEdit with renames', () => {
  const editWith = async (id: string, change: Partial<{ band: string; album: string }>) => {
    await load();
    selectTrack(id);
    startEdit();
    Object.assign(lib.draft!, change);
  };
  it('asks for confirmation first, then sends the group', async () => {
    await editWith('a', { album: 'Ordered' });
    await saveEdit();
    expect(lib.pendingRename?.kind).toBe('rename');
    expect(calls).toEqual([]);
    await saveEdit(true);
    const req = patchesOf('save_track')[0].request;
    expect(req.group.map((g: TrackPatch) => g.id)).toEqual(['b', 'c']);
    expect(req.group[0].album).toBe('Ordered');
    expect(lib.pendingRename).toBeNull();
    expect(lib.mode).toBe('view');
    expect(lib.library!.tracks.filter((t) => t.album === 'Ordered')).toHaveLength(3);
    expect(lib.expanded['b:band|a:ordered']).toBe(true);
    expect(logs).toContain('rename kind=album from="Order" to="Ordered" tracks=3 merged=false failed=0');
  });
  it('refuses clearing the band and stays in edit mode', async () => {
    await editWith('a', { band: '' });
    await saveEdit();
    expect(lib.errorMessage).toBe('The band name cannot be cleared.');
    expect(lib.mode).toBe('edit');
    expect(logs).toContain('rename refused reason=clear-band');
    expect(calls).toEqual([]);
  });
  it('a plain save sends no group', async () => {
    await editWith('a', {});
    lib.draft!.title = 'AA';
    await saveEdit();
    expect(patchesOf('save_track')[0].request.group).toBeUndefined();
    expect(lib.message).toBe('Saved.');
  });
  it('a no-album track joining an album sends its position', async () => {
    disk = disk.map((t) => (t.album === 'Order' ? { ...t, position: Number(t.id === 'a') + Number(t.id === 'b') * 2 + Number(t.id === 'c') * 3 } : t));
    await editWith('n', { album: 'order' });
    await saveEdit();
    expect(patchesOf('save_track')[0].request).toMatchObject({ position: 4 });
    expect(lib.pendingRename).toBeNull();
  });
});
