import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import EditorPane from './EditorPane.svelte';
import EditorView from '../../views/EditorView.svelte';
import { ed, resetEditor, syncSelection } from '$lib/editor-state.svelte';
import { lib, resetLibrary } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { EditorEvent, EditorSnapshot, TransportState } from '$lib/ipc';

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const stemTrack = FIXTURE_TRACKS[FIXTURE_TRACKS.length - 1];
const idle: TransportState = { playing: false, position_ms: 0, resume_pending: false, solo: null, clipping: false };
const snapshot = (id: string): EditorSnapshot => ({
  id, title: 'T', duration_ms: 187400, sample_rate: 44100, transport: { ...idle }, saving: null,
  variant: { id: 'backing', name: 'Backing', file: 'backings/backing.flac', exists: false },
  stems: [
    { name: 'guitar', gain_db: 0, unmuted: false },
    { name: 'drums', gain_db: 0, unmuted: false },
  ],
});

let calls: [string, Record<string, unknown>][];
let channel: Channel<EditorEvent>;
let handlers: Record<string, (a: Record<string, unknown>) => unknown>;
const names = () => calls.map((c) => c[0]);
const send = (e: EditorEvent) => channel.onmessage(e);
const flush = () => new Promise((r) => setTimeout(r, 0));

beforeEach(async () => {
  calls = [];
  handlers = {};
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (handlers[cmd]) return handlers[cmd](a);
    if (cmd === 'open_editor') { channel = a.events as Channel<EditorEvent>; return snapshot(a.id as string); }
    if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: [stemTrack], problems: [] };
    if (cmd === 'editor_set_stem') return { name: a.name, gain_db: a.gain, unmuted: a.unmuted };
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

async function active() {
  render(EditorPane);
  syncSelection(stemTrack);
  await waitFor(() => expect(ed.status).toBe('active'));
  await flush();
}
const btn = (n: string) => screen.getByRole('button', { name: n }) as HTMLButtonElement;
const thumb = (n: string) => screen.getByRole('slider', { name: n });

describe('states', () => {
  it('inactive: no lanes, Mix lane disabled, reason shown', async () => {
    render(EditorPane);
    syncSelection(null);
    await flush();
    expect(screen.getByText('Select a track with stems to build its backing track.')).toBeTruthy();
    expect(screen.queryByTestId('lane-guitar')).toBeNull();
    expect(btn('Play').disabled).toBe(true);
    expect(btn('Save').disabled).toBe(true);
  });

  it('error is an inline alert', async () => {
    handlers.open_editor = () => { throw 'boom'; };
    render(EditorPane);
    syncSelection(stemTrack);
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('boom'));
  });

  it('active: one lane per stem with capitalised labels and the save target', async () => {
    await active();
    expect(screen.getByTestId('lane-guitar').textContent).toContain('Guitar');
    expect(screen.getByTestId('lane-drums')).toBeTruthy();
    expect(screen.getByTestId('save-target').textContent).toBe('Save as: Backing (backings/backing.flac)');
  });
});

describe('controls', () => {
  it('lane Play checks Unmute, reads Solo, Mix reads Pause after the transport event', async () => {
    await active();
    handlers.editor_lane_play = () => ({ ...idle, playing: true, solo: 'guitar' });
    await fireEvent.click(btn('Play Guitar'));
    await waitFor(() => expect(btn('Solo Guitar')).toBeTruthy());
    const solo = btn('Solo Guitar');
    expect(solo.textContent!.trim()).toBe('Solo');
    expect(solo.getAttribute('aria-pressed')).toBe('true');
    await waitFor(() =>
      expect(screen.getByRole('checkbox', { name: 'Unmute Guitar' }).getAttribute('aria-checked')).toBe('true'),
    );
    expect(screen.getByTestId('solo-text').textContent).toBe('Solo: Guitar');
    expect(screen.getByTestId('lane-drums').getAttribute('data-dimmed')).toBe('true');
    send({ kind: 'transport', id: stemTrack.id, ...idle, playing: true, solo: 'guitar' });
    await waitFor(() => expect(btn('Pause')).toBeTruthy());
    expect(btn('Stop').disabled).toBe(false);
  });

  it('Mix Play needs a checked lane', async () => {
    await active();
    expect(btn('Play').disabled).toBe(true);
    expect(btn('Save').disabled).toBe(true);
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Unmute Drums' }));
    await waitFor(() => expect(btn('Play').disabled).toBe(false));
    expect(btn('Save').disabled).toBe(false);
    expect(calls.find((c) => c[0] === 'editor_set_stem')![1]).toMatchObject({ name: 'drums', unmuted: true });
  });

  it('a transport event updates the slider and the time text; clipping shows CLIP', async () => {
    await active();
    expect(screen.queryByTestId('clip-badge')).toBeNull();
    send({ kind: 'transport', id: stemTrack.id, ...idle, playing: true, position_ms: 5000, clipping: true });
    await waitFor(() => expect(screen.getByTestId('time-text').textContent).toBe('0:05.0'));
    expect(thumb('Position').getAttribute('aria-valuenow')).toBe('5000');
    expect(screen.getByTestId('clip-badge').textContent).toBe('CLIP');
  });

  it('dB text shows Off at the bottom, +12.0 dB at the top, and resets on click', async () => {
    await active();
    const g = thumb('Volume Guitar');
    g.focus();
    await fireEvent.keyDown(g, { key: 'End' });
    await waitFor(() => expect(screen.getByTestId('gain-text-guitar').textContent!.trim()).toBe('+12.0 dB'));
    await fireEvent.keyDown(g, { key: 'Home' });
    await waitFor(() => expect(screen.getByTestId('gain-text-guitar').textContent!.trim()).toBe('Off'));
    expect(g.getAttribute('aria-valuetext')).toBe('Off');
    await fireEvent.click(screen.getByTestId('gain-text-guitar'));
    await waitFor(() => expect(screen.getByTestId('gain-text-guitar').textContent!.trim()).toBe('0.0 dB'));
  });

  it('saving events show progress and a Cancel button; failures are alerts', async () => {
    await active();
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Unmute Drums' }));
    await waitFor(() => expect(btn('Save').disabled).toBe(false));
    handlers.save_backing = () => ({ ...snapshot(stemTrack.id), saving: 0 });
    await fireEvent.click(btn('Save'));
    send({ kind: 'saving', id: stemTrack.id, progress: 0.4 });
    await waitFor(() => expect(screen.getByTestId('save-percent').textContent).toBe('40%'));
    expect(btn('Save').disabled).toBe(true);
    await fireEvent.click(btn('Cancel'));
    expect(names()).toContain('cancel_backing_save');
    send({ kind: 'save-failed', id: stemTrack.id, message: 'disk full' });
    await waitFor(() => expect(screen.getByRole('alert').textContent).toBe('disk full'));
  });
});

describe('keys', () => {
  it('Space toggles play unless the focus is in a checkbox/slider/button', async () => {
    resetLibrary();
    render(EditorView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    lib.selectedId = stemTrack.id;
    await waitFor(() => expect(ed.status).toBe('active'));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Unmute Guitar' }));
    await waitFor(() => expect(btn('Play').disabled).toBe(false));
    const n = () => names().filter((x) => x === 'editor_play').length;
    await fireEvent.keyDown(screen.getByRole('slider', { name: 'Volume Guitar' }), { code: 'Space', key: ' ' });
    await fireEvent.keyDown(btn('Stop'), { code: 'Space', key: ' ' });
    await flush();
    expect(n()).toBe(0);
    await fireEvent.keyDown(document.body, { code: 'Space', key: ' ' });
    await waitFor(() => expect(n(), JSON.stringify(names())).toBe(1));
  });

  it('Ctrl+S saves when a lane is checked', async () => {
    render(EditorView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    lib.selectedId = stemTrack.id;
    await waitFor(() => expect(ed.status).toBe('active'));
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Unmute Guitar' }));
    await waitFor(() => expect(btn('Save').disabled).toBe(false));
    await fireEvent.keyDown(document.body, { code: 'KeyS', key: 's', ctrlKey: true });
    await waitFor(() => expect(names()).toContain('save_backing'));
  });
});
