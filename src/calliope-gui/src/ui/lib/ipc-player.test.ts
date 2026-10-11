import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  closePlayer, getPlayer, openPlayer, playerLeave, playerLoop, playerMetronome, playerNudge, playerPause,
  playerPlay, playerSeek, playerSelectSource, playerSkip, playerTempo, watchPlayer,
} from './ipc';
import { PLAYER_FIXTURE_TRACKS } from './fixture-tracks';

afterEach(() => clearMocks());

describe('player ipc wrappers', () => {
  it('send the command names and argument names', async () => {
    const calls: [string, Record<string, unknown>][] = [];
    mockIPC((cmd, args) => { calls.push([cmd, args as Record<string, unknown>]); return null; });
    await openPlayer('t1', () => {});
    await closePlayer();
    await getPlayer();
    await watchPlayer(() => {});
    await playerSelectSource('t1', 'stem:bass');
    await playerPlay('t1');
    await playerPause('t1');
    await playerSeek('t1', 1500);
    await playerNudge('t1', -100);
    await playerSkip('t1', 5000);
    await playerLeave('t1');
    await playerTempo('t1', { kind: 'set-bpm', bpm: 92.5 });
    await playerMetronome('t1', true, false);
    await playerLoop('t1', { kind: 'set', edge: 'start', value: { tick: 32 } });
    expect(calls.map((c) => c[0])).toEqual([
      'open_player', 'close_player', 'get_player', 'watch_player', 'player_select_source', 'player_play',
      'player_pause', 'player_seek', 'player_nudge', 'player_skip', 'player_leave', 'player_tempo',
      'player_metronome', 'player_loop',
    ]);
    const { events: e0, ...open } = calls[0][1];
    expect(open).toEqual({ id: 't1' });
    expect(e0).toBeDefined();
    expect(calls[1][1]).toEqual({});
    expect(calls[2][1]).toEqual({});
    expect(Object.keys(calls[3][1])).toEqual(['events']);
    expect(calls[4][1]).toEqual({ id: 't1', source: 'stem:bass' });
    expect(calls[5][1]).toEqual({ id: 't1' });
    expect(calls[6][1]).toEqual({ id: 't1' });
    expect(calls[7][1]).toEqual({ id: 't1', position: 1500 });
    expect(calls[8][1]).toEqual({ id: 't1', delta: -100 });
    expect(calls[9][1]).toEqual({ id: 't1', delta: 5000 });
    expect(calls[10][1]).toEqual({ id: 't1' });
    expect(calls[11][1]).toEqual({ id: 't1', action: { kind: 'set-bpm', bpm: 92.5 } });
    expect(calls[12][1]).toEqual({ id: 't1', on: true, countIn: false });
    expect(calls[13][1]).toEqual({ id: 't1', action: { kind: 'set', edge: 'start', value: { tick: 32 } } });
  });

  it('never sends a path', async () => {
    const calls: Record<string, unknown>[] = [];
    mockIPC((_c, args) => { calls.push((args ?? {}) as Record<string, unknown>); return null; });
    await openPlayer('t1', () => {});
    await playerPlay('t1');
    for (const a of calls) expect(Object.keys(a).filter((k) => /path|file|dir/i.test(k))).toEqual([]);
  });

  it('player fixtures cover a backing, stems only and an mp3', () => {
    expect(PLAYER_FIXTURE_TRACKS.map((t) => t.title)).toEqual(['Practice Song', 'Stems Only', 'MP3 Backing']);
    expect(PLAYER_FIXTURE_TRACKS[0].backings).toHaveLength(1);
    expect(PLAYER_FIXTURE_TRACKS[2].audio).toBe('backing.mp3');
  });
});
