// QA acceptance tests for gui-backing-track-editor (frontend, through the public views).
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import App from './App.svelte';
import { ui } from '$lib/app-state.svelte';
import { ed, resetEditor } from '$lib/editor-state.svelte';
import { load, resetLibrary, selectTrack } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import { formatPosition, parsePosition } from '$lib/time-format';
import { formatGain, gainToSlider, sliderToGain } from '$lib/gain-format';
import type { EditorEvent, EditorSnapshot, TrackRecord, TransportState } from '$lib/ipc';

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const plain = FIXTURE_TRACKS[0];
const stemA = FIXTURE_TRACKS[FIXTURE_TRACKS.length - 1];
const stemB: TrackRecord = { ...stemA, id: 'stem-b', title: 'Other stems' };
const idle: TransportState = { playing: false, position_ms: 0, resume_pending: false, solo: null, clipping: false };
const snap = (id: string): EditorSnapshot => ({
  id, title: 'T', duration_ms: 187400, sample_rate: 44100, transport: { ...idle }, saving: null,
  variant: { id: 'backing', name: 'Backing', file: 'backings/backing.flac', exists: false },
  stems: [
    { name: 'guitar', gain_db: 0, unmuted: false },
    { name: 'drums', gain_db: 0, unmuted: false },
  ],
});

let calls: [string, Record<string, unknown>][];
let channel: Channel<EditorEvent>;
const names = () => calls.map((c) => c[0]);
const logs = () => calls.filter((c) => c[0] === 'frontend_log').map((c) => String(c[1].message));
const flush = () => new Promise((r) => setTimeout(r, 0));
const btn = (n: string) => screen.getByRole('button', { name: n }) as HTMLButtonElement;
const key = async (digit: number) => {
  await fireEvent.keyDown(window, { code: `Digit${digit}`, altKey: true });
  await tick();
};

beforeEach(() => {
  calls = [];
  ui.view = 'library';
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'open_editor') { channel = a.events as Channel<EditorEvent>; return snap(a.id as string); }
    if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: [plain, stemA, stemB], problems: [] };
    if (cmd === 'editor_set_stem') return { name: a.name, gain_db: a.gain, unmuted: a.unmuted };
    if (cmd === 'editor_play') return { ...idle, playing: true };
    if (cmd === 'editor_pause') return { ...idle, position_ms: 4000 };
    if (cmd === 'editor_seek') return { ...idle, position_ms: a.position as number };
    if (cmd.startsWith('editor_')) return { ...idle };
    return null;
  });
  resetEditor();
  resetLibrary();
});
afterEach(() => {
  cleanup();
  clearMocks();
});

async function openEditorOn(t: TrackRecord) {
  render(App);
  await load();
  await key(3);
  selectTrack(t.id);
  await flush();
}

describe('owner decisions (formats)', () => {
  it('time shows M:SS.T, e.g. 3:07.4, floored to the tenth', () => {
    expect(formatPosition(187_400)).toBe('3:07.4');
    expect(formatPosition(187_499)).toBe('3:07.4');
    expect(formatPosition(0)).toBe('0:00.0');
    expect(formatPosition(60_000)).toBe('1:00.0');
    expect(formatPosition(599_900)).toBe('9:59.9');
    expect(parsePosition('3:07.4')).toBe(187_400);
    expect(parsePosition('3:07:4')).toBe(187_400);
    expect(parsePosition('1:75.0')).toBeNull();
    expect(parsePosition('-1')).toBeNull();
    expect(parsePosition('')).toBeNull();
  });

  it('volume is dB from -60 (Off) to +12 with one decimal', () => {
    expect(formatGain(null)).toBe('Off');
    expect(formatGain(-60)).toBe('Off');
    expect(formatGain(0)).toBe('0.0 dB');
    expect(formatGain(6.5)).toBe('+6.5 dB');
    expect(formatGain(-12)).toBe('-12.0 dB');
    expect(formatGain(12)).toBe('+12.0 dB');
    expect(sliderToGain(-60)).toBeNull();
    expect(sliderToGain(-59.5)).toBe(-59.5);
    expect(gainToSlider(null)).toBe(-60);
    expect(gainToSlider(99)).toBe(12);
  });
});

describe('through the whole app', () => {
  it('criterion 1/2: a track without stems leaves the pane inactive, one with stems activates it', async () => {
    await openEditorOn(plain);
    expect(ed.status).toBe('inactive');
    expect(names()).not.toContain('open_editor');
    expect(btn('Play').disabled).toBe(true);
    selectTrack(stemA.id);
    await waitFor(() => expect(ed.status).toBe('active'));
    expect(names().filter((n) => n === 'open_editor')).toHaveLength(1);
  });

  it('criteria 7/8/9/10: Play -> Pause + Stop enabled, Pause -> Play at the same position', async () => {
    await openEditorOn(stemA);
    await waitFor(() => expect(ed.status).toBe('active'));
    expect(btn('Stop').disabled).toBe(true);
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Unmute Guitar' }));
    await waitFor(() => expect(btn('Play').disabled).toBe(false));
    await fireEvent.click(btn('Play'));
    await waitFor(() => expect(btn('Pause')).toBeTruthy());
    expect(btn('Stop').disabled).toBe(false);
    channel.onmessage({ kind: 'transport', id: stemA.id, ...idle, playing: true, position_ms: 3200 });
    await waitFor(() => expect((screen.getByTestId('time-text') as HTMLInputElement).value).toBe('0:03.2'));
    await fireEvent.click(btn('Pause'));
    await waitFor(() => expect(btn('Play')).toBeTruthy());
    expect((screen.getByTestId('time-text') as HTMLInputElement).value).toBe('0:04.0');
    expect(btn('Stop').disabled).toBe(false); // position > 0
  });

  it('owner: leaving the Editor with Alt+1 while playing stops playback; returning shows it stopped', async () => {
    await openEditorOn(stemA);
    await waitFor(() => expect(ed.status).toBe('active'));
    channel.onmessage({ kind: 'transport', id: stemA.id, ...idle, playing: true, position_ms: 2000 });
    await flush();
    await key(1);
    await waitFor(() => expect(names()).toContain('editor_stop'));
    expect(logs()).toContain('editor leave stop');
    await key(3);
    await flush();
    expect(ed.status).toBe('active');
    expect(ed.transport.playing).toBe(false);
    expect(names().filter((n) => n === 'open_editor')).toHaveLength(1); // same track: no reload
  });

  it('owner: selecting another track in the Library, then the Editor, shows the new track and closes the old session', async () => {
    await openEditorOn(stemA);
    await waitFor(() => expect(ed.status).toBe('active'));
    await key(1);
    selectTrack(stemB.id);
    await key(3);
    await waitFor(() => expect(names().filter((n) => n === 'open_editor')).toHaveLength(2));
    expect(names()).toContain('close_editor');
    await waitFor(() => expect(ed.trackId).toBe(stemB.id));
  });

  it('criterion 11: typed time inside the range seeks, outside does not', async () => {
    await openEditorOn(stemA);
    await waitFor(() => expect(ed.status).toBe('active'));
    const f = screen.getByTestId('time-text') as HTMLInputElement;
    await fireEvent.input(f, { target: { value: '1:00.0' } });
    await fireEvent.keyDown(f, { key: 'Enter' });
    await flush();
    expect(calls.filter((c) => c[0] === 'editor_seek').at(-1)![1].position).toBe(60_000);
    const n = calls.filter((c) => c[0] === 'editor_seek').length;
    await fireEvent.input(f, { target: { value: '3:07.5' } }); // one tenth past 3:07.4
    await fireEvent.keyDown(f, { key: 'Enter' });
    await flush();
    expect(calls.filter((c) => c[0] === 'editor_seek')).toHaveLength(n);
    expect(f.getAttribute('aria-invalid')).toBe('true');
    // the exact end is allowed
    await fireEvent.input(f, { target: { value: '3:07.4' } });
    await fireEvent.keyDown(f, { key: 'Enter' });
    await flush();
    expect(calls.filter((c) => c[0] === 'editor_seek').at(-1)![1].position).toBe(187_400);
  });

  it('criteria 12/13: 3 wheel-ups send three -100 nudges, wheel-down sends +100', async () => {
    await openEditorOn(stemA);
    await waitFor(() => expect(ed.status).toBe('active'));
    const f = screen.getByTestId('time-text') as HTMLInputElement;
    for (let i = 0; i < 3; i++) f.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, cancelable: true, bubbles: true }));
    f.dispatchEvent(new WheelEvent('wheel', { deltaY: 120, cancelable: true, bubbles: true }));
    await flush();
    expect(calls.filter((c) => c[0] === 'editor_nudge').map((c) => c[1].delta)).toEqual([-100, -100, -100, 100]);
  });

  it('criterion 4: a Space press with nothing checked does not play; with a stem checked it does', async () => {
    await openEditorOn(stemA);
    await waitFor(() => expect(ed.status).toBe('active'));
    (document.activeElement as HTMLElement | null)?.blur();
    await fireEvent.keyDown(document.body, { code: 'Space', key: ' ' });
    await flush();
    expect(names()).not.toContain('editor_play');
  });

  it('criterion 16: Save stays disabled until a stem is unmuted, then calls save_backing once', async () => {
    await openEditorOn(stemA);
    await waitFor(() => expect(ed.status).toBe('active'));
    expect(btn('Save').disabled).toBe(true);
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Unmute Drums' }));
    await waitFor(() => expect(btn('Save').disabled).toBe(false));
    await fireEvent.click(btn('Save'));
    await waitFor(() => expect(names().filter((n) => n === 'save_backing')).toHaveLength(1));
  });
});
