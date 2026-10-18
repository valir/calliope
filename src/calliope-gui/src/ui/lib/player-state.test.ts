import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import {
  loop, beginNameEdit, cancelSectionBpm, cancelSectionName, commitSectionBpm, commitSectionName, jumpSection, setSectionFlags, editSectionBpm, leavePlayer, moveChange, pl, removeChange, scaleSection,
  selectSection, startAdd, targetSection, playLabel, resetPlayer, seek, selectSource, syncSelection, togglePlay } from './player-state.svelte';
import { resetLibrary } from './library-state.svelte';
import { PLAYER_FIXTURE_TRACKS } from './fixture-tracks';
import type { PlayerEvent, PlayerSnapshot, PlayerTransport, TempoSegment } from './ipc';

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
  loops: [], current_loop: null, loop_message: null,
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

  it('the loop log line carries end=<ms|none>', async () => {
    const lp = { id: 'l1', start: { ms: 100 }, end: { ms: 900 }, start_ms: 100, end_ms: 900, repeat: false, count_in: false };
    let cur: PlayerSnapshot = { ...snapshot(practice.id), loops: [lp], current_loop: 0 };
    handlers.player_loop = () => cur;
    syncSelection(practice);
    await flush();
    await loop({ kind: 'add' });
    expect(logs()).toContain('player loop add index=0 count=1 complete=true end=900');
    cur = { ...cur, loops: [{ ...lp, end: null, end_ms: null }] };
    await loop({ kind: 'add' });
    expect(logs()).toContain('player loop add index=0 count=1 complete=false end=none');
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
  const seg = (start_ms: number, bpm: number): TempoSegment => ({ start_ms, bpm, detected_ms: null, on_beat: true });
  const withSegs = (id: string, segs: TempoSegment[]): PlayerSnapshot => {
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
    expect(logs()).toContain('player sections segments=3 bpm=100,120,90 starts=0,2000,5000 metronome=1,1,1 count_in=0,0,0 named=0');
  });

  it('playback carrying the playhead into another section hands the target to it; Start jumps there', async () => {
    await open();
    const ev = (position_ms: number, tempo_index: number, playing = true) =>
      channels[0].onmessage({ kind: 'transport', id: practice.id, ...idle, playing, position_ms, tempo_index });
    // Section 2 selected on the waveform while the playhead is there too.
    ev(2500, 1, false);
    selectSection(1);
    ev(2600, 1);
    expect(targetSection()).toBe(1);
    ev(5200, 2);
    expect(pl.section).toBeNull();
    expect(targetSection()).toBe(2);
    ev(5300, 2, false);
    handlers.player_tempo = (a) => withSegs(a.id as string, three);
    await jumpSection('start');
    expect(tempoCalls()).toEqual([{ kind: 'jump-section', index: 2, at: 5000, edge: 'start' }]);
  });

  it('owner sequence: select section 2, Start, play across sections, pause: Start stays at the current section', async () => {
    const four = [seg(0, 100), seg(2000.01, 120), seg(5000.02, 90), seg(8000.03, 80)];
    await open(four);
    const tr = (position_ms: number, tempo_index: number, playing: boolean) => ({ ...idle, playing, position_ms, tempo_index });
    const ev = (t: PlayerTransport) => channels[0].onmessage({ kind: 'transport', id: practice.id, ...t });
    ev(tr(100, 0, false));
    selectSection(1);
    handlers.player_tempo = (a) => ({ ...withSegs(a.id as string, four), transport: tr(2000, 1, false) });
    await jumpSection('start');
    expect(pl.section).toBe(1);
    ev(tr(2000, 1, true));
    ev(tr(2600, 1, true));
    ev(tr(5100, 2, true));
    ev(tr(8200, 3, true));
    handlers.player_pause = () => tr(8300, 3, false);
    await togglePlay();
    expect(pl.transport.playing).toBe(false);
    expect(targetSection()).toBe(3);
    handlers.player_tempo = (a) => ({ ...withSegs(a.id as string, four), transport: tr(8000, 3, false) });
    await jumpSection('start');
    await jumpSection('start');
    const jumps = tempoCalls().filter((c) => (c as { kind: string }).kind === 'jump-section');
    expect(jumps.slice(-2)).toEqual([
      { kind: 'jump-section', index: 3, at: 8000.03, edge: 'start' },
      { kind: 'jump-section', index: 3, at: 8000.03, edge: 'start' },
    ]);
  });

  describe('selection follows the playhead through snapshots', () => {
    const four = [seg(0, 100), seg(2000, 120), seg(5000, 90), seg(8000, 80)];
    const tr = (position_ms: number, tempo_index: number, playing: boolean) => ({ ...idle, playing, position_ms, tempo_index });
    const reply = (segs: TempoSegment[], t: PlayerTransport) => {
      handlers.player_tempo = (a) => ({ ...withSegs(a.id as string, segs), transport: t });
    };

    it('(a) a snapshot-delivered index change during playback drops the selection', async () => {
      await open(four);
      channels[0].onmessage({ kind: 'transport', id: practice.id, ...tr(2100, 1, true) });
      selectSection(1);
      reply(four, tr(5100, 2, true));
      await setSectionFlags(true, false);
      expect(pl.section).toBeNull();
      expect(targetSection()).toBe(2);
    });

    it('(b) a section move whose snapshot changes the playhead index keeps the selection', async () => {
      await open(four);
      channels[0].onmessage({ kind: 'transport', id: practice.id, ...tr(5100, 2, false) });
      selectSection(2);
      reply([seg(0, 100), seg(2000, 120), seg(5200, 90), seg(8000, 80)], tr(5100, 1, false));
      moveChange(2, 1);
      await flush();
      expect(pl.section).toBe(2);
    });

    it('(c) jump-section into the selected section keeps it', async () => {
      await open(four);
      channels[0].onmessage({ kind: 'transport', id: practice.id, ...tr(100, 0, false) });
      selectSection(2);
      reply(four, tr(5000, 2, false));
      await jumpSection('start');
      expect(pl.section).toBe(2);
    });

    it('(d) Start reaches section 2 through a snapshot, playback crosses sections, pause, Start goes to the current one', async () => {
      await open(four);
      const ev = (t: PlayerTransport) => channels[0].onmessage({ kind: 'transport', id: practice.id, ...t });
      ev(tr(100, 0, false));
      selectSection(1);
      reply(four, tr(2000, 1, false));
      await jumpSection('start');
      ev(tr(2000, 1, true));
      ev(tr(5100, 2, true));
      ev(tr(8200, 3, true));
      handlers.player_pause = () => tr(8300, 3, false);
      await togglePlay();
      expect(targetSection()).toBe(3);
      reply(four, tr(8000, 3, false));
      await jumpSection('start');
      await jumpSection('start');
      const jumps = tempoCalls().filter((c) => (c as { kind: string }).kind === 'jump-section');
      expect(jumps.map((j) => (j as { index: number }).index)).toEqual([1, 3, 3]);
    });
  });

  it('a selection made while paused survives play within its section and a play from elsewhere', async () => {
    await open();
    const ev = (position_ms: number, tempo_index: number, playing: boolean) =>
      channels[0].onmessage({ kind: 'transport', id: practice.id, ...idle, playing, position_ms, tempo_index });
    ev(100, 0, false);
    selectSection(2);
    ev(100, 0, true);
    ev(900, 0, true);
    expect(targetSection()).toBe(2);
    ev(2100, 1, true);
    expect(targetSection()).toBe(1);
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

  it('a queued move keeps the at it was requested with when a remove lands in between', async () => {
    await open();
    let release: (v: unknown) => void = () => {};
    let n = 0;
    handlers.player_tempo = (a) => {
      if (n++ === 0) return new Promise((r) => { release = r; });
      return withSegs(a.id as string, three);
    };
    moveChange(2, 1);
    moveChange(1, 5); // other section: captured now, at 2000
    await flush();
    release(withSegs(practice.id, [seg(0, 100), seg(5000, 90)])); // a remove landed: section 1 is now the old 2
    await flush();
    expect(tempoCalls()).toEqual([
      { kind: 'move-change', index: 2, at: 5000, steps: 1 },
      { kind: 'move-change', index: 1, at: 2000, steps: 5 },
    ]);
  });

  it('cancelling the BPM overlay clears the section error', async () => {
    await open();
    pl.sectionError = 'Enter a BPM between 20 and 300';
    editSectionBpm(1);
    cancelSectionBpm();
    expect(pl.sectionError).toBe('');
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

  it('flags, name and jump send exact arguments and log; the name edit keeps its section', async () => {
    await open();
    const named = [seg(0, 100), { ...seg(2000, 120), name: 'Verse', metronome: false, count_in: true }, seg(5000, 90)];
    handlers.player_tempo = (a) => {
      const s = withSegs(a.id as string, named);
      s.transport.position_ms = 4500;
      return s;
    };
    pl.transport.tempo_index = 1;
    await setSectionFlags(false, true);
    pl.transport.tempo_index = 2;
    await jumpSection('end');
    pl.transport.tempo_index = 1;
    beginNameEdit();
    expect(pl.nameEdit).toEqual({ index: 1, at: 2000, draft: 'Verse' });
    pl.transport.tempo_index = 2;
    beginNameEdit();
    expect(pl.nameEdit?.index).toBe(1);
    pl.nameEdit!.draft = ' Chorus\t';
    await setSectionFlags(true, false);
    await commitSectionName();
    expect(pl.nameEdit).toBeNull();
    expect(tempoCalls()).toEqual([
      { kind: 'section-flags', index: 1, at: 2000, metronome: false, count_in: true },
      { kind: 'jump-section', index: 2, at: 5000, edge: 'end' },
      { kind: 'section-flags', index: 1, at: 2000, metronome: true, count_in: false },
      { kind: 'section-name', index: 1, at: 2000, name: 'Chorus' },
    ]);
    expect(logs()).toContain('player section flags index=1 metronome=false count_in=true');
    expect(logs()).toContain('player section jump index=2 edge=end position=4500');
    expect(logs()).toContain('player section name index=1 length=6');
    expect(logs()).toContain('player sections segments=3 bpm=100,120,90 starts=0,2000,5000 metronome=1,0,1 count_in=0,1,0 named=1');
  });

  it('an unchanged or cancelled name sends nothing; a stale refusal shows in sectionError', async () => {
    await open([seg(0, 100), { ...seg(2000, 120), name: 'Verse' }]);
    pl.transport.tempo_index = 1;
    beginNameEdit();
    pl.nameEdit!.draft = '  Verse ';
    await commitSectionName();
    beginNameEdit();
    pl.nameEdit!.draft = 'Other';
    cancelSectionName();
    await commitSectionName();
    expect(pl.nameEdit).toBeNull();
    expect(pl.snap?.tempo.segments[1].name).toBe('Verse');
    expect(tempoCalls()).toEqual([]);
    beginNameEdit();
    pl.nameEdit!.draft = 'Bridge';
    handlers.player_tempo = () => Promise.reject('The sections changed; try again');
    await commitSectionName();
    expect(pl.sectionError).toBe('The sections changed; try again');
    expect(pl.actionError).toBe('');
    syncSelection(stemsOnly);
    await flush();
    expect(pl.nameEdit).toBeNull();
  });
});
