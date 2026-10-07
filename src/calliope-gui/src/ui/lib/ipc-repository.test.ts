import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  chooseRepositoryRoot, deleteTrack, exportTablature, exportTrack, getRepository, listTracks,
  pickTablature, resetRepositoryRoot, saveTrack, setRepositoryRoot, type SaveTrackRequest,
} from './ipc';

afterEach(() => clearMocks());

describe('repository ipc wrappers', () => {
  it('send the command names and arguments', async () => {
    const calls: [string, unknown][] = [];
    mockIPC((cmd, args) => { calls.push([cmd, args]); return null; });
    const req: SaveTrackRequest = {
      id: 'abc',
      revision: '00ff',
      edits: { band: 'B', album: 'A', title: 'T', composers: ['C'], year: 1999, source_url: null, copyright: null },
      tablatures: [{ kind: 'keep', name: 'x.gp5' }, { kind: 'add', token: 'p1' }],
    };
    await getRepository();
    await chooseRepositoryRoot();
    await setRepositoryRoot('p2');
    await resetRepositoryRoot();
    await listTracks();
    await pickTablature('add');
    await saveTrack(req);
    await deleteTrack('abc', '00ff');
    await exportTrack('abc');
    await exportTablature('abc', 'x.gp5');
    expect(calls).toEqual([
      ['get_repository', {}],
      ['choose_repository_root', {}],
      ['set_repository_root', { token: 'p2' }],
      ['reset_repository_root', {}],
      ['list_tracks', {}],
      ['pick_tablature', { purpose: 'add' }],
      ['save_track', { request: req }],
      ['delete_track', { id: 'abc', revision: '00ff' }],
      ['export_track', { id: 'abc' }],
      ['export_tablature', { id: 'abc', name: 'x.gp5' }],
    ]);
  });

  it('returns null when a dialog is cancelled', async () => {
    mockIPC(() => null);
    expect(await pickTablature('update')).toBeNull();
    expect(await exportTrack('abc')).toBeNull();
  });
});
