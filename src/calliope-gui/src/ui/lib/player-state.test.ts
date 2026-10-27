import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import {
  loop, beginNameEdit, cancelSectionBpm, cancelSectionName, commitSectionBpm, commitSectionName, commitSectionOffset, beginOffsetEdit, cancelOffsetEdit, stepSection, jumpSection, findStart, detectBpm, cancelDetect, setSectionFlags, editSectionBpm, leavePlayer, moveChange, pl, removeChange, scaleSection,
  selectSection, startAdd, targetSection, playLabel, resetPlayer, seek, selectSource, syncSelection, togglePlay,
  speedStep, speedReset, speedSet, setTraining, speedLocked, shownSpeed } from './player-state.svelte';
import { resetLibrary } from './library-state.svelte';
import { PLAYER_FIXTURE_TRACKS } from './fixture-tracks';
import type { BpmDetectView, BpmResult, PlayerEvent, PlayerSnapshot, PlayerTransport, TempoSegment } from './ipc';

const [practice, stemsOnly] = PLAYER_FIXTURE_TRACKS;
const noAudio = { ...PLAYER_FIXTURE_TRACKS[2], audio: null };

const idle: PlayerTransport = {
  playing: false, counting_in: false, resume_pending: false, position_ms: 0,
  loop_playing: false, bpm: 0, played_bpm: 0, speed_pct: 100, tempo_index: null, training: null,
};
const snapshot = (id: string, source = 'backing:backing'): PlayerSnapshot => ({
  id, title: 'T',
  sources: [
    { key: 'backing:backing', label: 'Backing' }, { key: 'stem:guitar', label: 'Guitar' },
    { key: 'stem:drums', label: 'Drums' }, { key: 'original', label: 'Original mix' },
  ],
  source, duration_ms: 10000, sample_rate: 44100, peaks: [], mode: 'bpm',
  tempo: { beats_per_bar: 4, segments: [], multi: false }, speed_pct: 100,
  loops: [], current_loop: null, loop_message: null,
  transport: { ...idle }, note: null, read_only: false, bpm_detect: null,
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
      `player active id=${practice.id} source=backing:backing sources=4 mode=bpm loops=0 position=0 speed=100`,
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
    const lp = { id: 'l1', start: { ms: 100 }, end: { ms: 900 }, start_ms: 100, end_ms: 900, repeat: false, count_in: false, training: { on: false, start: 70, end: 100, step: 5, repeats: 2 } };
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

  it('findStart sends find-start and notes the new start; a reply after a track switch is ignored', async () => {
    await open();
    let release!: (s: PlayerSnapshot) => void;
    handlers.player_tempo = () => new Promise((res) => { release = res; });
    const run = findStart();
    expect(pl.findingStart).toBe(true);
    await findStart(); // busy: not sent twice
    release(withSegs(practice.id, [{ ...seg(1200, 100), found_ms: 1200 }, seg(2000, 120)]));
    await run;
    expect(tempoCalls()).toEqual([{ kind: 'find-start' }]);
    expect(pl.findingStart).toBe(false);
    expect(pl.sectionNote).toBe('First beat at 0:01.200');
    expect(logs()).toContain('player first-beat find');
    expect(logs()).toContain('player first-beat start=1200');
    // another section action clears the note
    selectSection(0);
    handlers.player_tempo = (a) => withSegs(a.id as string, three);
    await jumpSection('start');
    expect(pl.sectionNote).toBe('');
    // a reply arriving after a track switch is ignored
    handlers.player_tempo = () => new Promise((res) => { release = res; });
    const late = findStart();
    resetPlayer();
    release(withSegs(practice.id, [{ ...seg(900, 100), found_ms: 900 }]));
    await late;
    expect(pl.sectionNote).toBe('');
    expect(pl.snap).toBeNull();
  });

  describe('BPM detection', () => {
    const view = (serial = 1, phase: BpmDetectView['phase'] = 'checking'): BpmDetectView =>
      ({ serial, index: 1, at: 2000, from_ms: 10500, to_ms: 17700, source: 'backing:backing', phase });
    const running = (id: string, v: BpmDetectView | null = view()): PlayerSnapshot => ({ ...withSegs(id, three), bpm_detect: v });
    const result = (over: Partial<BpmResult> = {}): BpmResult =>
      ({ serial: 1, index: 1, outcome: 'applied', old_bpm: 120, new_bpm: 93.4, from_ms: 10500, to_ms: 17700, hint: null, message: null, ...over });
    const send = (e: PlayerEvent) => channels[0].onmessage(e);
    const detectCalls = (name = 'player_detect_bpm') => calls.filter((c) => c[0] === name).map((c) => c[1]);

    it('start sends the target section and its start; the reply sets pl.detect; phases follow the serial', async () => {
      await open();
      pl.transport.tempo_index = 1;
      handlers.player_detect_bpm = (a) => running(a.id as string);
      await detectBpm();
      expect(detectCalls()).toEqual([{ id: practice.id, index: 1, at: 2000 }]);
      expect(pl.detect?.phase).toBe('checking');
      expect(logs()).toContain('player detect start index=1');
      await detectBpm(); // running: not sent twice
      expect(detectCalls().length).toBe(1);
      send({ kind: 'bpm-detect', id: practice.id, detect: view(2, 'detecting') });
      expect(pl.detect?.phase).toBe('checking');
      send({ kind: 'bpm-detect', id: practice.id, detect: view(1, 'waiting') });
      expect(pl.detect?.phase).toBe('waiting');
      expect(logs()).toContain('player detect phase=waiting');
    });

    it('applied applies its snapshot, notes the status and clears the run', async () => {
      await open();
      selectSection(1);
      handlers.player_detect_bpm = (a) => running(a.id as string);
      await detectBpm();
      const after = withSegs(practice.id, [seg(0, 100), seg(2000, 93.4), seg(5000, 90)]);
      send({ kind: 'bpm-detected', id: practice.id, result: result(), snapshot: after });
      expect(pl.detect).toBeNull();
      expect(pl.snap?.tempo.segments[1].bpm).toBe(93.4);
      expect(pl.sectionError).toBe('');
      expect(pl.sectionNote).toBe('BPM 120.0 → 93.4, detected from 0:10.5 – 0:17.7');
      expect(logs()).toContain('player detected index=1 old=120 new=93.4 hint=none');
    });

    it('failed sets the error and keeps the segments; dropped sets the note', async () => {
      await open();
      selectSection(1);
      handlers.player_detect_bpm = (a) => running(a.id as string);
      await detectBpm();
      send({ kind: 'bpm-detected', id: practice.id, result: result({ outcome: 'failed', new_bpm: null, message: 'No beats found' }), snapshot: null });
      expect(pl.detect).toBeNull();
      expect(pl.sectionError).toBe('No beats found');
      expect(pl.snap?.tempo.segments.map((g) => g.bpm)).toEqual([100, 120, 90]);
      expect(logs()).toContain('player detect error message=No beats found');
      await detectBpm();
      send({ kind: 'bpm-detected', id: practice.id, result: result({ outcome: 'dropped', new_bpm: null, message: 'The section changed' }), snapshot: null });
      expect(pl.detect).toBeNull();
      expect(pl.sectionNote).toBe('The section changed');
      expect(logs()).toContain('player detect dropped index=1');
    });

    it('a refused start sets sectionError and no run', async () => {
      await open();
      selectSection(1);
      handlers.player_detect_bpm = () => { throw new Error('The section is too short'); };
      await detectBpm();
      expect(pl.sectionError).toBe('The section is too short');
      expect(pl.detect).toBeNull();
      expect(logs()).toContain('player detect error message=The section is too short');
    });

    it('cancel sends the command, clears the run and ignores a late phase of that serial', async () => {
      await open();
      selectSection(1);
      handlers.player_detect_bpm = (a) => running(a.id as string);
      await detectBpm();
      handlers.player_cancel_detect = (a) => running(a.id as string, null);
      await cancelDetect();
      expect(detectCalls('player_cancel_detect')).toEqual([{ id: practice.id }]);
      expect(pl.detect).toBeNull();
      expect(pl.sectionNote).toBe('BPM detection cancelled; the section is unchanged');
      expect(logs()).toContain('player detect cancel');
      send({ kind: 'bpm-detect', id: practice.id, detect: view(1, 'uploading') });
      expect(pl.detect).toBeNull();
    });

    it('guards: no tempo, read-only; events of another track or generation are ignored', async () => {
      await open([]);
      await detectBpm();
      expect(detectCalls()).toEqual([]);
      await open();
      handlers.open_player = (a) => ({ ...withSegs(a.id as string, three), read_only: true });
      resetPlayer();
      syncSelection(practice);
      await flush();
      selectSection(1);
      await detectBpm();
      expect(detectCalls()).toEqual([]);
      handlers.open_player = (a) => running(a.id as string);
      resetPlayer();
      syncSelection(practice);
      await flush();
      expect(pl.detect?.serial).toBe(1);
      send({ kind: 'bpm-detect', id: 'other', detect: view(1, 'waiting') });
      expect(pl.detect?.phase).toBe('checking');
      const old = channels[channels.length - 1];
      resetPlayer();
      old.onmessage({ kind: 'bpm-detect', id: practice.id, detect: view(1, 'waiting') });
      expect(pl.detect).toBeNull();
    });
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
  describe('start offset field and Prev / Next', () => {
    const S: TempoSegment[] = [
      { start_ms: 1200, bpm: 120, found_ms: 1200, detected_ms: null, on_beat: true },
      { start_ms: 3000, bpm: 120, detected_ms: 3000, on_beat: true },
      { start_ms: 4200, bpm: 120, detected_ms: null, on_beat: true },
    ];
    const snapS = (id: string): PlayerSnapshot => ({ ...withSegs(id, S), duration_ms: 5200 });
    const openS = async (): Promise<void> => {
      handlers.open_player = (a) => snapS(a.id as string);
      syncSelection(practice);
      await flush();
      handlers.player_tempo = (a) => snapS(a.id as string);
    };
    const at = (i: number) => {
      pl.transport.tempo_index = i;
      pl.section = null;
    };

    it('a valid text sends the absolute steps with at and resolves true', async () => {
      await openS();
      at(0);
      expect(await commitSectionOffset('+10')).toBe(true);
      expect(tempoCalls()).toEqual([{ kind: 'move-change', index: 0, at: 1200, steps: 10 }]);
      expect(logs()).toContain('player section offset index=0 text=+10 steps=10');
    });

    it('a too large value is clamped before it is sent', async () => {
      await openS();
      at(0);
      await commitSectionOffset('-9999');
      at(1);
      await commitSectionOffset('-9999');
      expect(tempoCalls()).toEqual([
        { kind: 'move-change', index: 0, at: 1200, steps: -154 },
        { kind: 'move-change', index: 1, at: 3000, steps: -166 },
      ]);
    });

    it('an invalid text sends nothing, resolves false and sets the message', async () => {
      await openS();
      at(0);
      expect(await commitSectionOffset('1.5')).toBe(false);
      expect(tempoCalls()).toEqual([]);
      expect(pl.sectionError).toBe('Type a whole number of 1/64 beats, like +10 or -8');
      expect(logs()).toContain('player section offset refused text=1.5');
    });

    it('the current value sends nothing and clears the message', async () => {
      await openS();
      at(0);
      pl.sectionError = 'old';
      expect(await commitSectionOffset('0')).toBe(true);
      expect(tempoCalls()).toEqual([]);
      expect(pl.sectionError).toBe('');
      expect(logs()).toContain('player section offset index=0 text=0 steps=0 unchanged');
    });

    it('the captured edit keeps its section when the playhead moves on', async () => {
      await openS();
      const ev = (tempo_index: number) => channels[0].onmessage({ kind: 'transport', id: practice.id, ...idle, tempo_index });
      ev(1);
      beginOffsetEdit();
      expect(pl.offsetEdit).toEqual({ index: 1, at: 3000 });
      ev(2);
      expect(targetSection()).toBe(2);
      await commitSectionOffset('-8');
      expect(tempoCalls()).toEqual([{ kind: 'move-change', index: 1, at: 3000, steps: -8 }]);
      cancelOffsetEdit();
      expect(pl.offsetEdit).toBeNull();
    });

    it('Next and Prev send the jump to the neighbour, like Start would', async () => {
      await openS();
      at(1);
      await stepSection(1);
      expect(pl.section).toBe(2);
      expect(logs().some((l) => /^player section jump index=2 edge=start position=\d+ via=next$/.test(l))).toBe(true);
      at(1);
      await stepSection(-1);
      expect(pl.section).toBe(0);
      expect(logs().some((l) => l.endsWith('via=prev'))).toBe(true);
      at(2);
      await jumpSection('start');
      at(1);
      await stepSection(1);
      expect(tempoCalls()).toEqual([
        { kind: 'jump-section', index: 2, at: 4200, edge: 'start' },
        { kind: 'jump-section', index: 0, at: 1200, edge: 'start' },
        { kind: 'jump-section', index: 2, at: 4200, edge: 'start' },
        { kind: 'jump-section', index: 2, at: 4200, edge: 'start' },
      ]);
    });

    it('Prev on the first and Next on the last section send nothing', async () => {
      await openS();
      at(0);
      await stepSection(-1);
      at(2);
      await stepSection(1);
      expect(tempoCalls()).toEqual([]);
    });

    it('a jump reply after a track switch is ignored', async () => {
      await openS();
      at(1);
      let release!: (s: PlayerSnapshot) => void;
      handlers.player_tempo = () => new Promise((res) => { release = res; });
      const run = stepSection(1);
      syncSelection(stemsOnly);
      await flush();
      release(snapS(practice.id));
      await run;
      expect(pl.section).toBeNull();
      expect(logs().some((l) => l.startsWith('player section jump'))).toBe(false);
    });
  });
});

describe('player speed and training', () => {
  const calls_ = (name: string) => calls.filter((c) => c[0] === name).map((c) => c[1]);
  const withSpeed = (id: string, pct: number, loopIdx: number | null = null): PlayerSnapshot => ({
    ...snapshot(id), speed_pct: pct, current_loop: loopIdx,
  });
  async function open(): Promise<void> {
    syncSelection(practice);
    await flush();
  }

  it('step, reset and typed speed send the action and log once; success clears the error', async () => {
    await open();
    pl.speedError = 'old';
    handlers.player_tempo = (a) => withSpeed(a.id as string, 95);
    await speedStep(-1, 'minus');
    expect(pl.speedError).toBe('');
    handlers.player_tempo = (a) => withSpeed(a.id as string, 100);
    await speedReset();
    handlers.player_tempo = (a) => withSpeed(a.id as string, 70);
    await speedSet(' 70 % ');
    await speedStep(1, 'key');
    expect(calls_('player_tempo').map((c) => c.action)).toEqual([
      { kind: 'speed-step', steps: -1 }, { kind: 'speed-reset' }, { kind: 'speed-set', percent: 70 }, { kind: 'speed-step', steps: 1 },
    ]);
    expect(logs().filter((l) => l.startsWith('player speed '))).toEqual([
      'player speed percent=95 via=minus', 'player speed percent=100 via=reset',
      'player speed percent=70 via=typed', 'player speed percent=70 via=key',
    ]);
    expect(shownSpeed()).toBe(70);
  });

  it('a refused typed value sends nothing and sets speedError; Rust errors go to speedError', async () => {
    await open();
    await speedSet('20');
    await speedSet('abc');
    expect(calls_('player_tempo')).toEqual([]);
    expect(pl.speedError).toBe('Enter a whole number from 25 to 200 %');
    expect(logs()).toContain('player speed refused text=20');
    expect(logs()).toContain('player speed refused text=abc');
    handlers.player_tempo = () => { throw new Error('nope'); };
    await speedStep(1, 'plus');
    expect(pl.speedError).toBe('nope');
    expect(pl.actionError).toBe('');
  });

  it('during a run the speed actions send nothing and the heard speed is shown', async () => {
    await open();
    pl.snap!.speed_pct = 90;
    expect(speedLocked()).toBe(false);
    expect(shownSpeed()).toBe(90);
    pl.transport.training = { loop: 0, speed_pct: 70, pass: 3, repeat: 1, of: 2 } as PlayerTransport['training'];
    pl.transport.speed_pct = 70;
    expect(speedLocked()).toBe(true);
    expect(shownSpeed()).toBe(70);
    await speedStep(-1, 'minus');
    await speedReset();
    await speedSet('50');
    expect(calls_('player_tempo')).toEqual([]);
  });

  it('setTraining sends the loop action for the current loop and logs; errors go to actionError', async () => {
    handlers.open_player = (a) => withSpeed(a.id as string, 100, 0);
    await open();
    const t = { on: true, start: 60, end: 70, step: 10, repeats: 1 };
    handlers.player_loop = (a) => withSpeed(a.id as string, 100, 0);
    await setTraining(t);
    expect(calls_('player_loop').map((c) => c.action)).toEqual([{ kind: 'training', ...t }]);
    expect(logs()).toContain('player training index=0 on=true start=60 end=70 step=10 repeats=1');
    handlers.player_loop = () => { throw new Error('bad plan'); };
    await setTraining(t);
    expect(pl.actionError).toBe('bad plan');
    expect(pl.speedError).toBe('');
  });

  it('the readout is logged only when its text changes', async () => {
    await open();
    const ev = (played_bpm: number, position_ms: number, speed_pct = 100): PlayerEvent =>
      ({ kind: 'transport', id: practice.id, ...idle, played_bpm, position_ms, speed_pct });
    const readouts = () => logs().filter((l) => l.startsWith('player readout'));
    channels[0].onmessage(ev(0, 10));
    expect(readouts()).toEqual([]);
    channels[0].onmessage(ev(60, 20, 50));
    channels[0].onmessage(ev(60, 30, 50));
    channels[0].onmessage(ev(72, 40, 60));
    channels[0].onmessage(ev(0, 50));
    expect(readouts()).toEqual([
      'player readout bpm=60 BPM speed=50', 'player readout bpm=72 BPM speed=60', 'player readout bpm=none speed=100',
    ]);
  });
});
