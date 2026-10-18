import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  closePlayer, getPlayer, openPlayer, playerLeave, playerLoop, playerNudge, playerPause,
  playerPlay, playerSeek, playerSelectSource, playerSkip, playerTempo, watchPlayer,
} from './ipc';
import type { PlayerEvent } from './ipc';
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
    await playerLoop('t1', { kind: 'set', edge: 'start', value: { tick: 32 } });
    await playerTempo('t1', { kind: 'speed-set', percent: 85 });
    await playerLoop('t1', { kind: 'training', on: true, start: 70, end: 100, step: 5, repeats: 2 });
    expect(calls.map((c) => c[0])).toEqual([
      'open_player', 'close_player', 'get_player', 'watch_player', 'player_select_source', 'player_play',
      'player_pause', 'player_seek', 'player_nudge', 'player_skip', 'player_leave', 'player_tempo',
      'player_loop', 'player_tempo', 'player_loop',
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
    expect(calls[12][1]).toEqual({ id: 't1', action: { kind: 'set', edge: 'start', value: { tick: 32 } } });
    expect(calls[13][1]).toEqual({ id: 't1', action: { kind: 'speed-set', percent: 85 } });
    expect(calls[14][1]).toEqual({ id: 't1', action: { kind: 'training', on: true, start: 70, end: 100, step: 5, repeats: 2 } });
  });

  it('types the closed event of a released session', async () => {
    const got: PlayerEvent[] = [];
    let ch: { onmessage: (e: PlayerEvent) => void } | undefined;
    mockIPC((_c, args) => { ch = (args as { events: typeof ch }).events; return null; });
    await watchPlayer((e) => got.push(e));
    ch!.onmessage({ kind: 'closed', id: 't1' });
    expect(got).toEqual([{ kind: 'closed', id: 't1' }]);
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

  it('player_tempo passes the section actions unchanged', async () => {
    const seen: unknown[] = [];
    mockIPC((cmd, args) => {
      seen.push([cmd, args]);
      return null;
    });
    await playerTempo('t1', { kind: 'move-change', index: 1, at: 8500, steps: -3 });
    await playerTempo('t1', { kind: 'scale-section', index: 0, at: 500, factor: 'double' });
    expect(seen).toEqual([
      ['player_tempo', { id: 't1', action: { kind: 'move-change', index: 1, at: 8500, steps: -3 } }],
      ['player_tempo', { id: 't1', action: { kind: 'scale-section', index: 0, at: 500, factor: 'double' } }],
    ]);
  });
  it('player_tempo passes the phase-2 section actions unchanged', async () => {
    const seen: unknown[] = [];
    mockIPC((cmd, args) => {
      seen.push([cmd, args]);
      return null;
    });
    await playerTempo('t1', { kind: 'section-flags', index: 1, at: 8500, metronome: false, count_in: true });
    await playerTempo('t1', { kind: 'section-name', index: 1, at: 8500, name: 'Verse' });
    await playerTempo('t1', { kind: 'jump-section', index: 0, at: 500, edge: 'end' });
    expect(seen).toEqual([
      ['player_tempo', { id: 't1', action: { kind: 'section-flags', index: 1, at: 8500, metronome: false, count_in: true } }],
      ['player_tempo', { id: 't1', action: { kind: 'section-name', index: 1, at: 8500, name: 'Verse' } }],
      ['player_tempo', { id: 't1', action: { kind: 'jump-section', index: 0, at: 500, edge: 'end' } }],
    ]);
  });
});
