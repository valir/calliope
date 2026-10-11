import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import { leavePlayer, pl, playLabel, resetPlayer, seek, selectSource, syncSelection, togglePlay } from './player-state.svelte';
import { resetLibrary } from './library-state.svelte';
import { PLAYER_FIXTURE_TRACKS } from './fixture-tracks';
import type { PlayerEvent, PlayerSnapshot, PlayerTransport } from './ipc';

const [practice, stemsOnly] = PLAYER_FIXTURE_TRACKS;
const noAudio = { ...PLAYER_FIXTURE_TRACKS[2], audio: null };

const idle: PlayerTransport = {
  playing: false, counting_in: false, resume_pending: false, position_ms: 0,
  loop_playing: false, bpm: 0, tempo_index: null,
};
const snapshot = (id: string, source = 'backing:backing'): PlayerSnapshot => ({
  id, title: 'T',
  sources: [
    { key: 'backing:backing', label: 'Backing' }, { key: 'stem:guitar', label: 'Guitar' },
    { key: 'stem:drums', label: 'Drums' }, { key: 'original', label: 'Original mix' },
  ],
  source, duration_ms: 10000, sample_rate: 44100, peaks: [], mode: 'bpm',
  tempo: { beats_per_bar: 4, segments: [], multi: false }, speed: 1,
  metronome: { on: false, count_in: false }, loops: [], current_loop: null, loop_message: null,
  transport: { ...idle }, can_change_speed: false, note: null,
});

type Call = [string, Record<string, unknown>];
let calls: Call[];
let channels: Channel<PlayerEvent>[];
let handlers: Record<string, (args: Record<string, unknown>) => unknown>;
const names = () => calls.map((c) => c[0]);
const flush = () => new Promise((r) => setTimeout(r, 0));
const logs = () => calls.filter((c) => c[0] === 'frontend_log').map((c) => c[1].message as string);

beforeEach(() => {
  calls = [];
  channels = [];
  handlers = {};
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'open_player') channels.push(a.events as Channel<PlayerEvent>);
    if (handlers[cmd]) return handlers[cmd](a);
    if (cmd === 'open_player') return snapshot(a.id as string);
    if (cmd === 'player_select_source') return snapshot(a.id as string, a.source as string);
    if (cmd.startsWith('player_')) return { ...idle };
    if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: [practice], problems: [] };
    return null;
  });
  resetPlayer();
  resetLibrary();
});
afterEach(() => clearMocks());

describe('player state', () => {
  it('no selection and unplayable tracks are inactive', async () => {
    syncSelection(null);
    await flush();
    expect(pl.status).toBe('inactive');
    expect(pl.message).toBe('Select a track to play it.');
    syncSelection(noAudio);
    await flush();
    expect(pl.message).toBe(`${noAudio.title} has no audio to play.`);
    expect(names()).not.toContain('open_player');
  });

  it('a playable track opens and logs active', async () => {
    syncSelection(practice);
    expect(pl.status).toBe('loading');
    await flush();
    expect(pl.status).toBe('active');
    expect(logs()).toContain(
      `player active id=${practice.id} source=backing:backing sources=4 mode=bpm loops=0 position=0`,
    );
  });

  it('switching tracks closes and opens, and ignores a late snapshot', async () => {
    let release: (v: unknown) => void = () => {};
    handlers.open_player = (a) =>
      a.id === practice.id ? new Promise((r) => { release = r; }) : snapshot(a.id as string);
    syncSelection(practice);
    await flush();
    syncSelection(stemsOnly);
    await flush();
    release(snapshot(practice.id));
    await flush();
    expect(pl.status).toBe('active');
    expect(pl.trackId).toBe(stemsOnly.id);
    expect(names()).toContain('close_player');
  });

  it('selectSource calls the command and the button reads Play', async () => {
    syncSelection(practice);
    await flush();
    await selectSource('stem:drums');
    expect(names()).toContain('player_select_source');
    expect(logs()).toContain('player source key=stem:drums');
    expect(playLabel()).toBe('Play');
    expect(pl.snap?.source).toBe('stem:drums');
  });

  it('a transport event updates position and bpm', async () => {
    syncSelection(practice);
    await flush();
    channels[0].onmessage({ kind: 'transport', id: practice.id, ...idle, playing: true, position_ms: 1200, bpm: 120 });
    expect(pl.transport.position_ms).toBe(1200);
    expect(pl.transport.bpm).toBe(120);
    expect(playLabel()).toBe('Pause');
    channels[0].onmessage({ kind: 'transport', id: 'other', ...idle, position_ms: 5 });
    expect(pl.transport.position_ms).toBe(1200);
  });

  it('leaving while playing calls player_leave', async () => {
    syncSelection(practice);
    await flush();
    leavePlayer();
    expect(names()).not.toContain('player_leave');
    await togglePlay();
    handlers.player_play = () => ({ ...idle, playing: true });
    pl.transport.playing = true;
    leavePlayer();
    await flush();
    expect(names()).toContain('player_leave');
    expect(logs()).toContain('player leave pause');
    expect(pl.transport.playing).toBe(false);
  });

  it('rapid seeks keep at most 2 in flight, last value last', async () => {
    syncSelection(practice);
    await flush();
    let inflight = 0;
    let peak = 0;
    handlers.player_seek = async (a) => {
      inflight++;
      peak = Math.max(peak, inflight);
      await new Promise((r) => setTimeout(r, 5));
      inflight--;
      return { ...idle, position_ms: a.position as number };
    };
    for (let i = 1; i <= 20; i++) seek(i * 100);
    await new Promise((r) => setTimeout(r, 100));
    const seeks = calls.filter((c) => c[0] === 'player_seek');
    expect(peak).toBeLessThanOrEqual(2);
    expect(seeks.length).toBeLessThan(20);
    expect(seeks[seeks.length - 1][1].position).toBe(2000);
  });
});
