import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import {
  canPlay, ed, lanePlay, leaveEditor, resetEditor, save, saveEnabled, setGain, setUnmuted, stopEnabled,
  syncSelection,
} from './editor-state.svelte';
import { resetLibrary } from './library-state.svelte';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { EditorEvent, EditorSnapshot, TrackRecord, TransportState } from './ipc';

const plainTrack = FIXTURE_TRACKS[0];
const stemTrack = FIXTURE_TRACKS[FIXTURE_TRACKS.length - 1];
const otherStemTrack: TrackRecord = { ...stemTrack, id: 'other', title: 'Other' };

const idle: TransportState = { playing: false, position_ms: 0, resume_pending: false, solo: null, clipping: false };
const snapshot = (id: string, unmuted = false): EditorSnapshot => ({
  id, title: 'T', duration_ms: 10000, sample_rate: 44100, transport: { ...idle }, saving: null,
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

  it('ignores a superseded load', async () => {
    handlers.open_editor = () => { throw 'superseded'; };
    syncSelection(stemTrack);
    await flush();
    expect(ed.status).toBe('loading');
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
