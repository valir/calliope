import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  cancelBackingSave, closeEditor, editorEndSolo, editorLanePlay, editorNudge, editorPause, editorPlay,
  editorSeek, editorSetStem, editorStop, getEditor, openEditor, saveBacking, watchEditor,
} from './ipc';
import { EDITOR_FIXTURE_TRACKS } from './fixture-tracks';

afterEach(() => clearMocks());

describe('editor ipc wrappers', () => {
  it('send the command names and argument names', async () => {
    const calls: [string, Record<string, unknown>][] = [];
    mockIPC((cmd, args) => { calls.push([cmd, args as Record<string, unknown>]); return null; });
    await openEditor('t1', () => {});
    await closeEditor();
    await getEditor();
    await watchEditor(() => {});
    await editorPlay('t1');
    await editorLanePlay('t1', 'drums');
    await editorEndSolo('t1');
    await editorPause('t1');
    await editorStop('t1');
    await editorSeek('t1', 1500);
    await editorNudge('t1', -100);
    await editorSetStem('t1', 'drums', -3.5, true);
    await editorSetStem('t1', 'bass', null, false);
    await saveBacking('t1');
    await cancelBackingSave('t1');
    const plain = (i: number) => {
      const { events: _e, ...rest } = calls[i][1];
      return rest;
    };
    expect(calls.map((c) => c[0])).toEqual([
      'open_editor', 'close_editor', 'get_editor', 'watch_editor', 'editor_play', 'editor_lane_play',
      'editor_end_solo', 'editor_pause', 'editor_stop', 'editor_seek', 'editor_nudge', 'editor_set_stem',
      'editor_set_stem', 'save_backing', 'cancel_backing_save',
    ]);
    expect(plain(0)).toEqual({ id: 't1' });
    expect(calls[0][1].events).toBeDefined();
    expect(plain(3)).toEqual({});
    expect(calls[3][1].events).toBeDefined();
    expect(calls[1][1]).toEqual({});
    expect(calls[4][1]).toEqual({ id: 't1' });
    expect(calls[5][1]).toEqual({ id: 't1', name: 'drums' });
    expect(calls[6][1]).toEqual({ id: 't1' });
    expect(calls[7][1]).toEqual({ id: 't1' });
    expect(calls[8][1]).toEqual({ id: 't1' });
    expect(calls[9][1]).toEqual({ id: 't1', position: 1500 });
    expect(calls[10][1]).toEqual({ id: 't1', delta: -100 });
    expect(calls[11][1]).toEqual({ id: 't1', name: 'drums', gain: -3.5, unmuted: true });
    expect(calls[12][1]).toEqual({ id: 't1', name: 'bass', gain: null, unmuted: false });
    expect(calls[13][1]).toEqual({ id: 't1' });
    expect(calls[14][1]).toEqual({ id: 't1' });
  });

  it('never sends a path', async () => {
    const calls: Record<string, unknown>[] = [];
    mockIPC((_c, args) => { calls.push((args ?? {}) as Record<string, unknown>); return null; });
    await openEditor('t1', () => {});
    await saveBacking('t1');
    for (const a of calls) expect(Object.keys(a).filter((k) => /path|file|dir/i.test(k))).toEqual([]);
  });

  it('editor fixtures carry backings and a missing stem', () => {
    expect(EDITOR_FIXTURE_TRACKS[0].backings[0].file).toBe('backings/backing.flac');
    expect(EDITOR_FIXTURE_TRACKS[1].missing).toEqual(['stems/drums.flac']);
  });
});
