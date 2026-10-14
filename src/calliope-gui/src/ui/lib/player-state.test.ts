import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import {
  cancelSectionBpm, commitSectionBpm, editSectionBpm, leavePlayer, moveChange, pl, removeChange, scaleSection,
  selectSection, startAdd, targetSection, playLabel, resetPlayer, seek, selectSource, syncSelection, togglePlay } from './player-state.svelte';
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

  it('a closed event bumps reopen and the next sync of the same track opens it again', async () => {
    syncSelection(practice);
    await flush();
    const opens = () => names().filter((n) => n === 'open_player').length;
    expect(opens()).toBe(1);
    const before = pl.reopen;
    channels[0].onmessage({ kind: 'closed', id: 'other' });
    expect(pl.reopen).toBe(before);
    syncSelection(practice);
    expect(opens()).toBe(1);
    channels[0].onmessage({ kind: 'closed', id: practice.id });
    expect(pl.reopen).toBe(before + 1);
    syncSelection(practice);
    await flush();
    expect(opens()).toBe(2);
    expect(names()).not.toContain('close_player');
    expect(pl.status).toBe('active');
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

describe('player sections', () => {
  const seg = (start_ms: number, bpm: number) => ({ start_ms, bpm, detected_ms: null, on_beat: true });
  const withSegs = (id: string, segs: ReturnType<typeof seg>[]): PlayerSnapshot => {
    const s = snapshot(id);
    s.tempo = { beats_per_bar: 4, segments: segs, multi: segs.length > 1 };
    return s;
  };
  const three = [seg(0, 100), seg(2000, 120), seg(5000, 90)];
  const tempoCalls = () => calls.filter((c) => c[0] === 'player_tempo').map((c) => c[1].action);

  async function open(segs = three): Promise<void> {
    handlers.open_player = (a) => withSegs(a.id as string, segs);
    syncSelection(practice);
    await flush();
  }

  it('sends the exact arguments, with at from the snapshot, to the selection else tempo_index', async () => {
    await open();
    pl.transport.tempo_index = 2;
    expect(targetSection()).toBe(2);
    handlers.player_tempo = (a) => withSegs(a.id as string, three);
    await scaleSection(2);
    selectSection(1);
    expect(targetSection()).toBe(1);
    await scaleSection(0.5);
    editSectionBpm(1);
    await commitSectionBpm(77);
    await removeChange();
    expect(tempoCalls()).toEqual([
      { kind: 'scale-section', index: 2, at: 5000, factor: 'double' },
      { kind: 'scale-section', index: 1, at: 2000, factor: 'halve' },
      { kind: 'section-bpm', index: 1, at: 2000, bpm: 77 },
      { kind: 'remove-change', index: 1, at: 2000 },
    ]);
    expect(logs()).toContain('player section remove index=1');
    expect(logs()).toContain('player sections segments=3 bpm=100,120,90 starts=0,2000,5000');
  });

  it('add captures the playhead, selects the new section; remove selects the joined one', async () => {
    await open();
    pl.transport.position_ms = 3000;
    startAdd();
    pl.transport.position_ms = 4000;
    expect(pl.bpmEdit).toEqual({ addAt: 3000 });
    handlers.player_tempo = (a) =>
      withSegs(a.id as string, [seg(0, 100), seg(2000, 120), seg(3008, 80), seg(5000, 90)]);
    await commitSectionBpm(80);
    expect(tempoCalls()).toEqual([{ kind: 'add-change', position: 3000, bpm: 80 }]);
    expect(pl.section).toBe(2);
    expect(pl.bpmEdit).toBeNull();
    handlers.player_tempo = (a) => withSegs(a.id as string, three);
    await removeChange();
    expect(pl.section).toBe(1);
  });

  it('selection is cleared on track change and clamped after a shorter snapshot', async () => {
    await open();
    selectSection(2);
    handlers.player_tempo = (a) => withSegs(a.id as string, [seg(0, 100), seg(2000, 120)]);
    selectSection(1);
    await scaleSection(2);
    expect(pl.section).toBe(1);
    pl.section = 2;
    await scaleSection(2);
    expect(pl.section).toBe(1);
    startAdd();
    syncSelection(stemsOnly);
    expect(pl.section).toBeNull();
    expect(pl.bpmEdit).toBeNull();
    cancelSectionBpm();
  });

  it('three moves while one is pending make exactly two calls, the second with the latest steps and new at', async () => {
    await open();
    let release: (v: unknown) => void = () => {};
    let n = 0;
    handlers.player_tempo = (a) => {
      if (n++ === 0) return new Promise((r) => { release = r; });
      return withSegs(a.id as string, three);
    };
    moveChange(1, 1);
    moveChange(1, 2);
    moveChange(1, 3);
    await flush();
    expect(tempoCalls()).toHaveLength(1);
    release(withSegs(practice.id, [seg(0, 100), seg(2010, 120), seg(5000, 90)]));
    await flush();
    expect(tempoCalls()).toEqual([
      { kind: 'move-change', index: 1, at: 2000, steps: 1 },
      { kind: 'move-change', index: 1, at: 2010, steps: 3 },
    ]);
  });

  it('a refusal sets sectionError only, and the next success clears it', async () => {
    await open();
    selectSection(1);
    handlers.player_tempo = () => Promise.reject('Section 2 is too short');
    await scaleSection(2);
    expect(pl.sectionError).toBe('Section 2 is too short');
    expect(pl.actionError).toBe('');
    expect(pl.snap?.tempo.segments).toHaveLength(3);
    expect(pl.section).toBe(1);
    handlers.player_tempo = (a) => withSegs(a.id as string, three);
    await scaleSection(2);
    expect(pl.sectionError).toBe('');
  });

  it('a stale answer after a track change is dropped', async () => {
    await open();
    selectSection(1);
    let release: (v: unknown) => void = () => {};
    handlers.player_tempo = () => new Promise((r) => { release = r; });
    handlers.open_player = (a) => snapshot(a.id as string);
    const p = scaleSection(2);
    syncSelection(stemsOnly);
    await flush();
    release(withSegs(practice.id, [seg(0, 1)]));
    await p;
    expect(pl.trackId).toBe(stemsOnly.id);
    expect(pl.snap?.tempo.segments).toHaveLength(0);
    expect(logs().some((l) => l.startsWith('player section scale'))).toBe(false);
  });
});
