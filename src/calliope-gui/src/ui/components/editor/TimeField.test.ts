import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import TimeField from './TimeField.svelte';
import { ed, resetEditor, syncSelection } from '$lib/editor-state.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { EditorEvent, EditorSnapshot, TransportState } from '$lib/ipc';

const stemTrack = FIXTURE_TRACKS[FIXTURE_TRACKS.length - 1];
const idle: TransportState = { playing: false, position_ms: 0, resume_pending: false, solo: null, clipping: false };
const snapshot = (id: string): EditorSnapshot => ({
  id, title: 'T', duration_ms: 187400, sample_rate: 44100, transport: { ...idle }, saving: null,
  variant: { id: 'backing', name: 'Backing', file: 'backings/backing.flac', exists: false }, stems: [{ name: 'guitar', gain_db: 0, unmuted: false }],
});

let calls: [string, Record<string, unknown>][];
let channel: Channel<EditorEvent>;
const flush = () => new Promise((r) => setTimeout(r, 0));
const field = () => screen.getByLabelText('Position (minutes:seconds.tenths)') as HTMLInputElement;
const seeks = () => calls.filter((c) => c[0] === 'editor_seek');
const nudges = () => calls.filter((c) => c[0] === 'editor_nudge');

beforeEach(async () => {
  calls = [];
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'open_editor') { channel = a.events as Channel<EditorEvent>; return snapshot(a.id as string); }
    if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: [stemTrack], problems: [] };
    if (cmd.startsWith('editor_')) return { ...idle };
    return null;
  });
  resetEditor();
  resetLibrary();
  render(TimeField);
  syncSelection(stemTrack);
  await waitFor(() => expect(ed.status).toBe('active'));
  await flush();
});
afterEach(() => {
  cleanup();
  clearMocks();
});

async function type(text: string, key = 'Enter') {
  await fireEvent.input(field(), { target: { value: text } });
  await fireEvent.keyDown(field(), { key });
  await flush();
}

describe('TimeField', () => {
  it.each(['0:05.0', '0:05:0'])('typing %s + Enter seeks to 5000', async (t) => {
    await type(t);
    expect(seeks().at(-1)?.[1].position).toBe(5000);
    expect(field().getAttribute('aria-invalid')).toBeNull();
  });

  it.each(['9:99.9', '5:00.0', 'abc'])('%s is invalid and calls nothing', async (t) => {
    await type(t);
    expect(seeks()).toHaveLength(0);
    expect(field().getAttribute('aria-invalid')).toBe('true');
    expect(screen.getByTestId('time-error')).toBeTruthy();
  });

  it('Escape reverts', async () => {
    await type('9:99.9');
    await fireEvent.keyDown(field(), { key: 'Escape' });
    expect(field().value).toBe('0:00.0');
    expect(field().getAttribute('aria-invalid')).toBeNull();
  });

  it('wheel up nudges -100, wheel down +100, default-prevented', async () => {
    const up = new WheelEvent('wheel', { deltaY: -100, cancelable: true, bubbles: true });
    field().dispatchEvent(up);
    await flush();
    expect(up.defaultPrevented).toBe(true);
    expect(nudges().at(-1)?.[1].delta).toBe(-100);
    const down = new WheelEvent('wheel', { deltaY: 53, cancelable: true, bubbles: true });
    field().dispatchEvent(down);
    await flush();
    expect(down.defaultPrevented).toBe(true);
    expect(nudges().at(-1)?.[1].delta).toBe(100);
  });

  it('a transport event does not overwrite unsaved text', async () => {
    await fireEvent.input(field(), { target: { value: '1:2' } });
    channel.onmessage({ kind: 'transport', id: stemTrack.id, ...idle, position_ms: 9000 });
    ed.transport.position_ms = 9000;
    await flush();
    expect(field().value).toBe('1:2');
  });
});
