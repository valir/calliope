import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import PlayerPane from './PlayerPane.svelte';
import { pl, resetPlayer, syncSelection } from '$lib/player-state.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { PLAYER_FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { PlayerEvent, PlayerLoop, PlayerSnapshot, PlayerTransport } from '$lib/ipc';

const [practice] = PLAYER_FIXTURE_TRACKS;
const idle: PlayerTransport = {
  playing: false, counting_in: false, resume_pending: false, position_ms: 0,
  loop_playing: false, bpm: 0, tempo_index: null,
};
const loop1: PlayerLoop = {
  id: 'loop-1', start: { tick: 64 }, end: { tick: 192 }, start_ms: 2000, end_ms: 10000, repeat: false, count_in: false,
};
// A second loop without an end.
const loop3: PlayerLoop = { id: 'loop-3', start: { tick: 32 }, end: null, start_ms: 1000, end_ms: null, repeat: false, count_in: false };
const withTempo = (): PlayerSnapshot => ({
  id: practice.id, title: 'T', sources: [{ key: 'backing:backing', label: 'Backing' }], source: 'backing:backing',
  duration_ms: 20000, sample_rate: 44100, peaks: [], mode: 'bpm',
  tempo: { beats_per_bar: 4, segments: [{ start_ms: 0, bpm: 120 }], multi: true }, speed: 1,
  metronome: { on: false, count_in: false }, loops: [loop1, { ...loop1, id: 'loop-2' }, loop3], current_loop: 0,
  loop_message: null, transport: { ...idle, bpm: 120, tempo_index: 0 }, can_change_speed: false, note: null,
});
const timestamps = (): PlayerSnapshot => ({
  ...withTempo(), mode: 'time', tempo: { beats_per_bar: 4, segments: [], multi: false },
  loops: [{ ...loop1, start: { ms: 2500 }, start_ms: 2500, end: { ms: 6000 }, end_ms: 6000 }],
  transport: { ...idle },
});

let calls: [string, Record<string, unknown>][];
let channel: Channel<PlayerEvent>;
let snap: PlayerSnapshot;
const flush = () => new Promise((r) => setTimeout(r, 0));
const of = (cmd: string) => calls.filter((c) => c[0] === cmd).map((c) => c[1]);
const group = (name: string) => within(screen.getByRole('group', { name }));
const btn = (g: string, name: string) => group(g).getByRole('button', { name }) as HTMLButtonElement;
const input = (name: string) => screen.getByRole('textbox', { name }) as HTMLInputElement;

async function open(s: PlayerSnapshot) {
  snap = s;
  syncSelection(practice);
  await waitFor(() => expect(pl.status).toBe('active'));
  await flush();
}
async function type(el: HTMLInputElement, text: string) {
  await fireEvent.input(el, { target: { value: text } });
  await fireEvent.keyDown(el, { key: 'Enter' });
  await flush();
}

beforeEach(() => {
  calls = [];
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'open_player') { channel = a.events as Channel<PlayerEvent>; return snap; }
    if (cmd === 'player_tempo' || cmd === 'player_metronome' || cmd === 'player_loop') return snap;
    if (cmd.startsWith('player_')) return { ...idle };
    if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: [practice], problems: [] };
    return null;
  });
  resetPlayer();
  resetLibrary();
  render(PlayerPane);
});
afterEach(() => {
  cleanup();
  clearMocks();
});

describe('BPM and metronome', () => {
  it('shows the BPM and the V indicator; Enter sets it', async () => {
    await open(withTempo());
    expect(input('BPM').value).toBe('120');
    expect((screen.getByRole('checkbox', { name: 'V' }) as HTMLInputElement).checked).toBe(true);
    await type(input('BPM'), '90');
    expect(of('player_tempo')[0]).toMatchObject({ action: { kind: 'set-bpm', bpm: 90 } });
  });

  it('V is unchecked for a single tempo; 19 is invalid and sends nothing', async () => {
    await open({ ...withTempo(), tempo: { beats_per_bar: 4, segments: [{ start_ms: 0, bpm: 120 }], multi: false } });
    expect((screen.getByRole('checkbox', { name: 'V' }) as HTMLInputElement).checked).toBe(false);
    await type(input('BPM'), '19');
    expect(input('BPM').getAttribute('aria-invalid')).toBe('true');
    expect(of('player_tempo')).toHaveLength(0);
    expect(screen.getByRole('alert').textContent).toBe('Enter a BPM between 20 and 300, or 0 to clear');
  });

  it('metronome buttons toggle', async () => {
    await open(withTempo());
    const on = btn('Metronome', 'On/Off');
    expect(on.getAttribute('aria-pressed')).toBe('false');
    await fireEvent.click(on);
    await fireEvent.click(btn('Metronome', 'Count In'));
    expect(of('player_metronome')).toMatchObject([{ on: true, countIn: false }, { on: false, countIn: true }]);
  });

  it('timestamp mode disables metronome, -, + and Reset', async () => {
    await open(timestamps());
    expect(input('BPM').value).toBe('0');
    for (const n of ['On/Off', 'Count In']) expect(btn('Metronome', n).disabled).toBe(true);
    for (const n of ['Slower', 'Faster', 'Reset', 'Detect']) expect(btn('BPM', n).disabled).toBe(true);
  });
});

describe('loops', () => {
  it('shows the section box and the bars; typing a bar sets a tick', async () => {
    await open(withTempo());
    expect(input('Loop').value).toBe('1/3');
    expect(input('bar1').value).toBe('2.1.0');
    expect(input('bar2').value).toBe('4.1.0');
    await type(input('bar1'), '3.1.0');
    expect(of('player_loop')[0]).toMatchObject({ action: { kind: 'set', edge: 'start', value: { tick: 128 } } });
  });

  it('timestamp mode shows and sends milliseconds', async () => {
    await open(timestamps());
    expect(input('bar1').value).toBe('0:02.500');
    await type(input('bar2'), '0:07.000');
    expect(of('player_loop')[0]).toMatchObject({ action: { kind: 'set', edge: 'end', value: { ms: 7000 } } });
  });

  it('an incomplete current loop empties bar2 and the slider and disables its buttons', async () => {
    await open({ ...withTempo(), current_loop: 2 });
    expect(input('bar2').value).toBe('');
    expect(input('bar2').placeholder).toBe('end');
    for (const n of ['Play', 'Loop On', 'Count-In']) expect(btn('Loop playback', n).disabled).toBe(true);
    expect(screen.getByTestId('loop-slider').getAttribute('aria-valuenow')).toBeNull();
  });

  it('buttons send their actions', async () => {
    await open(withTempo());
    for (const [g, n] of [
      ['Loop range', 'Mark start'], ['Loop range', 'Mark end'], ['Loop list', 'Add loop'], ['Loop list', 'Remove loop'],
      ['Loop list', 'Prev'], ['Loop list', 'Next'], ['Loop playback', 'Play'], ['Loop playback', 'Loop On'],
      ['Loop playback', 'Count-In'],
    ]) await fireEvent.click(btn(g, n));
    await flush();
    expect(of('player_loop').map((c) => c.action)).toEqual([
      { kind: 'mark', edge: 'start' }, { kind: 'mark', edge: 'end' }, { kind: 'add' }, { kind: 'remove' },
      { kind: 'select', delta: -1 }, { kind: 'select', delta: 1 }, { kind: 'play' },
      { kind: 'flags', repeat: true, count_in: false }, { kind: 'flags', repeat: false, count_in: true },
    ]);
  });

  it('Stop while the loop plays; the status line shows loop_message', async () => {
    snap = withTempo();
    await open({ ...withTempo(), loop_message: 'The end must be after the start', transport: { ...idle, loop_playing: true, playing: true } });
    expect(screen.getByTestId('loop-status').textContent).toBe('The end must be after the start');
    await fireEvent.click(btn('Loop playback', 'Stop'));
    expect(of('player_loop')[0]).toMatchObject({ action: { kind: 'stop' } });
  });

  it('the loop slider is not focusable and shows 50 %', async () => {
    await open({ ...withTempo(), transport: { ...idle, position_ms: 6000 } });
    const s = screen.getByRole('slider', { name: 'Position in the loop' });
    expect(s.getAttribute('aria-valuenow')).toBe('50');
    expect(s.hasAttribute('tabindex')).toBe(false);
  });
});

describe('playback', () => {
  it('Play reads Pause while counting in; Rew/FF skip', async () => {
    await open(withTempo());
    expect(btn('Playback', 'Play')).toBeTruthy();
    channel.onmessage({ kind: 'transport', id: practice.id, ...idle, counting_in: true });
    await waitFor(() => expect(btn('Playback', 'Pause')).toBeTruthy());
    await fireEvent.click(btn('Playback', 'Rew'));
    await fireEvent.click(btn('Playback', 'FF'));
    await flush();
    expect(of('player_skip').map((c) => c.delta)).toEqual([-5000, 5000]);
  });

  it('the moving position slider does not seek by itself; a key on it does', async () => {
    await open(withTempo());
    for (const p of [3530, 3790, 3849, 3911]) {
      channel.onmessage({ kind: 'transport', id: practice.id, ...idle, playing: true, position_ms: p });
      await flush();
    }
    expect(of('player_seek')).toHaveLength(0);
    const slider = screen.getByRole('slider', { name: 'Position' });
    slider.focus();
    await fireEvent.keyDown(slider, { key: 'ArrowRight' });
    await fireEvent.keyUp(slider, { key: 'ArrowRight' });
    await flush();
    expect(of('player_seek').length).toBeGreaterThan(0);
  });
});
