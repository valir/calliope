import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import {
  cancelDetect, canDetect, canPlay, detect, detectHint, ed, lanePlay, moveChange, reloadTempo, leaveEditor, resetEditor, save, saveEnabled, seek, setGain, setUnmuted, stopEnabled,
  syncSelection, togglePlay,
} from './editor-state.svelte';
import { ui } from './app-state.svelte';
import { resetLibrary } from './library-state.svelte';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { EditorEvent, EditorSnapshot, TempoSnap, TrackRecord, TransportState } from './ipc';

const plainTrack = FIXTURE_TRACKS[0];
const stemTrack = FIXTURE_TRACKS[FIXTURE_TRACKS.length - 1];
const otherStemTrack: TrackRecord = { ...stemTrack, id: 'other', title: 'Other' };

const idle: TransportState = { playing: false, position_ms: 0, resume_pending: false, solo: null, clipping: false };
const snapshot = (id: string, unmuted = false): EditorSnapshot => ({
  id, title: 'T', duration_ms: 10000, sample_rate: 44100, transport: { ...idle }, saving: null,
  peaks: [], tempo: { beats_per_bar: 4, segments: [], multi: false }, detect: null,
  variant: { id: 'backing', name: 'Backing', file: 'backings/backing.flac', exists: false },
  stems: [
    { name: 'guitar', gain_db: 0, unmuted },
    { name: 'drums', gain_db: 0, unmuted: false },
  ],
});

type Call = [string, Record<string, unknown>];
let calls: Call[];
let channels: Channel<EditorEvent>[];
let handlers: Record<string, (args: Record<string, unknown>) => unknown>;
const names = () => calls.map((c) => c[0]);
const flush = () => new Promise((r) => setTimeout(r, 0));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

beforeEach(() => {
  calls = [];
  channels = [];
  handlers = {};
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'open_editor') channels.push(a.events as Channel<EditorEvent>);
    if (handlers[cmd]) return handlers[cmd](a);
    if (cmd === 'open_editor') return snapshot(a.id as string);
    if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: [stemTrack], problems: [] };
    if (cmd.startsWith('editor_')) return { ...idle };
    return null;
  });
  resetEditor();
  resetLibrary();
});
afterEach(() => clearMocks());

const logs = () => calls.filter((c) => c[0] === 'frontend_log').map((c) => c[1].message as string);

describe('activation', () => {
  it('a track without stems is inactive and opens nothing', async () => {
    syncSelection(plainTrack);
    await flush();
    expect(ed.status).toBe('inactive');
    expect(names()).not.toContain('open_editor');
    expect(logs()).toContain(`editor inactive id=${plainTrack.id}`);
    expect(ed.message).toContain('has no stems');
  });

  it('a track with a missing stem file is inactive', async () => {
    syncSelection({ ...stemTrack, missing: ['stems/drums.flac'] });
    await flush();
    expect(ed.status).toBe('inactive');
    expect(ed.message).toBe('Stem file stems/drums.flac is missing.');
  });

  it('a track with stems opens and becomes active', async () => {
    syncSelection(stemTrack);
    expect(ed.status).toBe('loading');
    await flush();
    expect(names()).toContain('open_editor');
    expect(ed.status).toBe('active');
    expect(ed.stems).toHaveLength(2);
    expect(logs()).toContain(`editor active id=${stemTrack.id} stems=2`);
  });

  it('switching tracks closes, opens and ignores the late first snapshot', async () => {
    let release!: (s: EditorSnapshot) => void;
    handlers.open_editor = (a) =>
      a.id === stemTrack.id ? new Promise((r) => { release = r; }) : snapshot(a.id as string);
    syncSelection(stemTrack);
    await flush();
    syncSelection(otherStemTrack);
    await flush();
    release(snapshot(stemTrack.id));
    await flush();
    expect(names().filter((n) => n === 'close_editor' || n === 'open_editor')).toEqual([
      'open_editor', 'close_editor', 'open_editor',
    ]);
    expect(ed.trackId).toBe('other');
    expect(ed.status).toBe('active');
  });

  it('playback stops on a track change', async () => {
    syncSelection(stemTrack);
    await flush();
    syncSelection(plainTrack);
    await flush();
    expect(names()).toContain('close_editor');
    expect(ed.status).toBe('inactive');
    expect(ed.transport.playing).toBe(false);
  });

  it('asks again when its own open was superseded by a stale one', async () => {
    let n = 0;
    handlers.open_editor = (a) => {
      if (++n === 1) throw 'superseded';
      return snapshot(a.id as string);
    };
    syncSelection(stemTrack);
    await sleep(20);
    expect(names().filter((c) => c === 'open_editor')).toHaveLength(2);
    expect(ed.status).toBe('active');
  });

  it('gives up with an error when every open is superseded', async () => {
    handlers.open_editor = () => { throw 'superseded'; };
    syncSelection(stemTrack);
    await sleep(20);
    expect(names().filter((c) => c === 'open_editor')).toHaveLength(3);
    expect(ed.status).toBe('error');
  });

  it('ignores a superseded open of an older selection', async () => {
    let release: () => void = () => {};
    handlers.open_editor = (a) => {
      if (a.id === stemTrack.id) return new Promise((_, rej) => { release = () => rej('superseded'); });
      return snapshot(a.id as string);
    };
    syncSelection(stemTrack);
    await flush();
    syncSelection(otherStemTrack);
    await flush();
    release();
    await sleep(10);
    expect(ed.status).toBe('active');
    expect(ed.trackId).toBe('other');
    expect(names().filter((c) => c === 'open_editor')).toHaveLength(2);
  });

  it('shows other load errors', async () => {
    handlers.open_editor = () => { throw 'boom'; };
    syncSelection(stemTrack);
    await flush();
    expect(ed.status).toBe('error');
    expect(ed.message).toContain('boom');
  });
});

describe('solo', () => {
  it('lanePlay sends the name and shows the lane unmuted and soloed', async () => {
    syncSelection(stemTrack);
    await flush();
    handlers.editor_lane_play = () => ({ ...idle, playing: true, solo: 'guitar' });
    await lanePlay('guitar');
    const call = calls.find((c) => c[0] === 'editor_lane_play')!;
    expect(call[1]).toEqual({ id: stemTrack.id, name: 'guitar' });
    expect(ed.stems.find((l) => l.name === 'guitar')!.unmuted).toBe(true);
    expect(ed.transport.solo).toBe('guitar');
    expect(logs()).toContain('editor solo name=guitar');
  });
});

describe('play errors', () => {
  it('a Play that cannot open the output shows why, and a later Play clears it', async () => {
    syncSelection(stemTrack);
    await flush();
    handlers.editor_set_stem = (a) => ({ name: a.name, gain_db: 0, unmuted: a.unmuted });
    await setUnmuted('guitar', true);
    handlers.editor_play = () => {
      throw 'The audio output cannot be opened: Device or resource busy';
    };
    await togglePlay();
    expect(ed.audioError).toBe('The audio output cannot be opened: Device or resource busy');
    expect(ed.transport.playing).toBe(false);
    expect(logs()).toContain('error editor editor_play: The audio output cannot be opened: Device or resource busy');
    handlers.editor_play = () => ({ ...idle, playing: true });
    await togglePlay();
    expect(ed.audioError).toBe('');
    expect(ed.transport.playing).toBe(true);
  });
});

describe('enables', () => {
  it('canPlay needs a checked stem unless playing', async () => {
    syncSelection(stemTrack);
    await flush();
    expect(canPlay()).toBe(false);
    ed.stems[0].unmuted = true;
    expect(canPlay()).toBe(true);
    ed.stems[0].unmuted = false;
    ed.transport.playing = true;
    expect(canPlay()).toBe(true);
  });

  it('stopEnabled follows playing and position; saveEnabled needs a checked stem', async () => {
    syncSelection(stemTrack);
    await flush();
    expect(stopEnabled()).toBe(false);
    ed.transport.position_ms = 500;
    expect(stopEnabled()).toBe(true);
    ed.transport.position_ms = 0;
    ed.transport.playing = true;
    expect(stopEnabled()).toBe(true);
    expect(saveEnabled()).toBe(false);
    ed.stems[1].unmuted = true;
    expect(saveEnabled()).toBe(true);
  });

  it('a never-saved track starts with every stem unchecked', async () => {
    syncSelection(stemTrack);
    await flush();
    expect(ed.stems.every((s) => !s.unmuted)).toBe(true);
    expect(canPlay()).toBe(false);
  });
});

describe('coalescing', () => {
  it('20 rapid setGain calls: at most 2 in flight, the last value last', async () => {
    syncSelection(stemTrack);
    await flush();
    let inFlight = 0;
    let maxInFlight = 0;
    handlers.editor_set_stem = async (a) => {
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      await sleep(20);
      inFlight--;
      return { name: a.name, gain_db: a.gain, unmuted: a.unmuted };
    };
    for (let i = 1; i <= 20; i++) setGain('guitar', -i);
    await sleep(120);
    const sent = calls.filter((c) => c[0] === 'editor_set_stem');
    expect(maxInFlight).toBe(1);
    expect(sent.length).toBeLessThanOrEqual(2);
    expect(sent[sent.length - 1][1].gain).toBe(-20);
    expect(ed.stems[0].gain_db).toBe(-20);
  });

  it('unchecking the soloed stem clears the solo locally', async () => {
    syncSelection(stemTrack);
    await flush();
    ed.stems[0].unmuted = true;
    ed.transport.solo = 'guitar';
    setUnmuted('guitar', false);
    expect(ed.transport.solo).toBeNull();
    await flush();
    const c = calls.find((x) => x[0] === 'editor_set_stem')!;
    expect(c[1]).toMatchObject({ name: 'guitar', unmuted: false });
  });
});

describe('leaving the view', () => {
  it('stops playback and logs it', async () => {
    syncSelection(stemTrack);
    await flush();
    ed.transport.playing = true;
    leaveEditor();
    await flush();
    expect(names()).toContain('editor_stop');
    expect(logs()).toContain('editor leave stop');
    expect(ed.transport.playing).toBe(false);
  });

  it('does nothing when already stopped at 0', async () => {
    syncSelection(stemTrack);
    await flush();
    leaveEditor();
    await flush();
    expect(names()).not.toContain('editor_stop');
  });
});

describe('seeking', () => {
  const at = (position_ms: number, playing = false) =>
    channels[0].onmessage({ kind: 'transport', id: stemTrack.id, ...idle, playing, position_ms });

  it('keeps the slider at the sought position until an event reflects the seek', async () => {
    let finish: (t: TransportState) => void = () => {};
    handlers.editor_seek = () => new Promise<TransportState>((r) => { finish = r; });
    syncSelection(stemTrack);
    await flush();
    at(1000, true);
    expect(ed.transport.position_ms).toBe(1000);
    seek(6000);
    expect(ed.transport.position_ms).toBe(6000);
    at(1100, true); // stale event from before the seek, while the call runs
    expect(ed.transport.position_ms).toBe(6000);
    finish({ ...idle, playing: true, position_ms: 6000 });
    await flush();
    at(1200, true); // stale event that was already in flight
    expect(ed.transport.position_ms).toBe(6000);
    at(6100, true); // the first event after the seek
    expect(ed.transport.position_ms).toBe(6100);
    at(6200, true);
    expect(ed.transport.position_ms).toBe(6200);
  });

  it('a later seek during a running one keeps the newest position', async () => {
    const finish: ((t: TransportState) => void)[] = [];
    handlers.editor_seek = () => new Promise<TransportState>((r) => { finish.push(r); });
    syncSelection(stemTrack);
    await flush();
    seek(3000);
    seek(4000);
    at(500);
    expect(ed.transport.position_ms).toBe(4000);
    finish[0]({ ...idle, position_ms: 3000 });
    await flush();
    expect(ed.transport.position_ms).toBe(4000);
    finish[1]({ ...idle, position_ms: 4000 });
    await flush();
    expect(ed.transport.position_ms).toBe(4000);
  });

  it('a stop or end of track after a seek is not held back', async () => {
    syncSelection(stemTrack);
    await flush();
    at(1000, true);
    seek(6000);
    await flush();
    at(0, false); // playback ended and stopped
    expect(ed.transport.position_ms).toBe(0);
    expect(ed.transport.playing).toBe(false);
  });
});

describe('events and save', () => {
  it('applies transport events and ignores other ids', async () => {
    syncSelection(stemTrack);
    await flush();
    channels[0].onmessage({ kind: 'transport', id: stemTrack.id, ...idle, playing: true, position_ms: 1200 });
    expect(ed.transport.position_ms).toBe(1200);
    channels[0].onmessage({ kind: 'transport', id: 'zzz', ...idle, position_ms: 9 });
    expect(ed.transport.position_ms).toBe(1200);
  });

  it('a saved event with clipping sets the warning and reloads the library', async () => {
    syncSelection(stemTrack);
    await flush();
    ed.stems[0].unmuted = true;
    handlers.save_backing = () => ({ ...snapshot(stemTrack.id), saving: 0.1 });
    await save();
    expect(ed.saving).toBe(0.1);
    channels[0].onmessage({
      kind: 'saved', id: stemTrack.id, file: 'backings/backing.flac', clipped_samples: 88200,
      track: stemTrack, warnings: [],
    });
    await flush();
    expect(ed.saving).toBeNull();
    expect(ed.saveMessage).toContain('88200 samples were clipped');
    expect(ed.saveMessage).toContain('about 1.0 s');
    expect(names()).toContain('list_tracks');
    expect(logs()).toContain(`editor saved id=${stemTrack.id} file=backings/backing.flac clipped=88200`);
  });

  it('save-failed shows the message', async () => {
    syncSelection(stemTrack);
    await flush();
    channels[0].onmessage({ kind: 'save-failed', id: stemTrack.id, message: 'disk full' });
    expect(ed.saveError).toBe('disk full');
  });
});

describe('waveform, tempo and detection', () => {
  const tempo = (offset = 0): TempoSnap => ({
    beats_per_bar: 4, multi: true,
    segments: [
      { start_ms: 0, bpm: 120 },
      { start_ms: 6000 + offset * (500 / 64), bpm: 90, detected_ms: 6000 },
    ],
  });
  const open = async () => {
    syncSelection(stemTrack);
    await flush();
    ed.stems[0].unmuted = true;
  };
  const emit = (e: object) => channels[0].onmessage({ id: stemTrack.id, ...e } as EditorEvent);

  afterEach(() => { ui.edgeAi = 'not-configured'; });

  it('canDetect truth table and hints', async () => {
    ui.edgeAi = 'connected';
    expect(canDetect()).toBe(false);
    await open();
    expect(canDetect()).toBe(true);
    expect(detectHint()).toBe('');
    // the checked stems and volumes do not matter
    ed.stems = ed.stems.map((l) => ({ ...l, unmuted: false, gain_db: null }));
    expect(canDetect()).toBe(true);
    expect(detectHint()).toBe('');
    ui.edgeAi = 'unreachable';
    expect(canDetect()).toBe(true);
    ui.edgeAi = 'not-configured';
    expect(canDetect()).toBe(false);
    expect(detectHint()).toBe('Set the edge-AI server in Settings to detect the BPM');
    ui.edgeAi = 'connected';
    ed.detect = { phase: 'mixing', progress: 0.4 };
    expect(canDetect()).toBe(false);
    expect(detectHint()).toBe('');
  });

  it('takes peaks and tempo from the snapshot and peaks events', async () => {
    handlers.open_editor = (a) => ({ ...snapshot(a.id as string), peaks: [0.1, 0.5], tempo: tempo() });
    await open();
    expect(ed.peaks).toEqual([0.1, 0.5]);
    expect(ed.tempo.segments).toHaveLength(2);
    emit({ kind: 'peaks', peaks: [0, 0.25, 0.75] });
    expect(ed.peaks).toEqual([0, 0.25, 0.75]);
    expect(logs()).toContain('editor waveform peaks=3 max=0.75');
    channels[0].onmessage({ kind: 'peaks', id: 'zzz', peaks: [1] });
    expect(ed.peaks).toHaveLength(3);
  });

  it('detect starts, shows phases and ends with the tempo and a message', async () => {
    ui.edgeAi = 'connected';
    await open();
    handlers.editor_detect = () => ({ ...snapshot(stemTrack.id), detect: { phase: 'checking', progress: null } });
    await detect();
    expect(ed.detect).toEqual({ phase: 'checking', progress: null });
    expect(logs()).toContain('editor detect start');
    emit({ kind: 'detect', phase: 'uploading', progress: 0.7 });
    emit({ kind: 'detect', phase: 'uploading', progress: 0.8 });
    expect(ed.detect).toEqual({ phase: 'uploading', progress: 0.8 });
    expect(logs().filter((l) => l === 'editor detect phase=uploading')).toHaveLength(1);
    ed.saveMessage = 'Saved.';
    emit({ kind: 'detected', tempo: tempo(), beats: 400, replaced: true });
    expect(ed.detect).toBeNull();
    expect(ed.tempo.segments).toHaveLength(2);
    expect(ed.detectMessage).toBe('Detected 400 beats: 2 tempo sections. The previous tempo map was replaced.');
    expect(ed.saveMessage).toBe('');
    expect(logs()).toContain('editor detected segments=2 bpm=120,90');
    emit({ kind: 'detected', tempo: tempo(), beats: 400, replaced: false });
    expect(ed.detectMessage).not.toContain('replaced');
    emit({ kind: 'saved', file: 'f', clipped_samples: 0, track: stemTrack, warnings: [] });
    expect(ed.detectMessage).toBe('');
    expect(ed.saveMessage).toBe('Saved.');
  });

  it('detect-failed and detect-cancelled end the job', async () => {
    await open();
    ed.detect = { phase: 'waiting', progress: null };
    emit({ kind: 'detect-failed', message: 'server down' });
    expect(ed.detect).toBeNull();
    expect(ed.detectError).toBe('server down');
    expect(logs()).toContain('editor detect error message=server down');
    ed.detect = { phase: 'waiting', progress: null };
    emit({ kind: 'detect-cancelled' });
    expect(ed.detect).toBeNull();
    expect(ed.detectError).toBe('');
    expect(ed.detectMessage).toBe('Detection cancelled.');
  });

  it('a failing editor_detect call shows the error; cancel calls the backend', async () => {
    ui.edgeAi = 'connected';
    await open();
    handlers.editor_detect = () => { throw 'no mix'; };
    await detect();
    expect(ed.detectError).toBe('no mix');
    expect(ed.detect).toBeNull();
    ed.detect = { phase: 'detecting', progress: null };
    await cancelDetect();
    expect(names()).toContain('editor_cancel_detect');
    expect(logs()).toContain('editor detect cancel');
  });

  it('moveChange coalesces to the latest absolute steps and logs the applied ones', async () => {
    await open();
    ed.tempo = tempo();
    const sent: number[] = [];
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    handlers.editor_tempo = async (a) => {
      const act = a.action as { kind: string; steps: number };
      sent.push(act.steps);
      if (sent.length === 1) await gate;
      return { ...snapshot(stemTrack.id), tempo: tempo(Math.min(act.steps, 3)) };
    };
    moveChange(1, 1);
    moveChange(1, 2);
    moveChange(1, 5);
    await flush();
    expect(sent).toEqual([1]);
    release();
    await sleep(20);
    expect(sent).toEqual([1, 5]);
    expect(ed.tempo.segments[1].start_ms).toBeCloseTo(6000 + 3 * (500 / 64));
    expect(logs()).toContain('editor tempo move index=1 steps=1 applied=1');
    expect(logs()).toContain('editor tempo move index=1 steps=5 applied=3');
  });

  it('moveChange on the first bar logs the steps from the found start', async () => {
    await open();
    ed.tempo = tempo();
    handlers.editor_tempo = async () => ({
      ...snapshot(stemTrack.id),
      tempo: { ...tempo(), segments: [{ start_ms: 500 + 3 * (500 / 64), bpm: 120, found_ms: 500 }, tempo().segments[1]] },
    });
    moveChange(0, 3);
    await sleep(20);
    expect(logs()).toContain('editor tempo move index=0 steps=3 applied=3');
  });

  it('moveChange is ignored during a detection; reloadTempo refreshes the map', async () => {
    await open();
    ed.detect = { phase: 'mixing', progress: 0 };
    moveChange(1, 4);
    await flush();
    expect(names()).not.toContain('editor_tempo');
    handlers.editor_tempo = () => ({ ...snapshot(stemTrack.id), tempo: tempo() });
    await reloadTempo();
    expect(ed.tempo.segments).toHaveLength(2);
    expect(logs()).toContain('editor tempo reload segments=2');
  });

  it('late results of a closed session are dropped', async () => {
    await open();
    let release!: () => void;
    const gate = new Promise<void>((r) => { release = r; });
    handlers.editor_tempo = async () => { await gate; return { ...snapshot(stemTrack.id), tempo: tempo() }; };
    const p = reloadTempo();
    syncSelection(otherStemTrack);
    release();
    await p;
    expect(ed.tempo.segments).toHaveLength(0);
    emit({ kind: 'peaks', peaks: [1] });
    expect(ed.peaks).toEqual([]);
  });
});
