import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import PlayerPane from './PlayerPane.svelte';
import { pl, resetPlayer, selectSection, syncSelection } from '$lib/player-state.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { PLAYER_FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { PlayerEvent, PlayerLoop, PlayerSnapshot, PlayerTransport } from '$lib/ipc';

const [practice] = PLAYER_FIXTURE_TRACKS;
const idle: PlayerTransport = {
  playing: false, counting_in: false, resume_pending: false, position_ms: 0,
  loop_playing: false, bpm: 0, played_bpm: 0, speed_pct: 100, tempo_index: null, training: null,
};
const loop1: PlayerLoop = {
  id: 'loop-1', start: { tick: 64 }, end: { tick: 192 }, start_ms: 2000, end_ms: 10000, repeat: false, count_in: false, training: { on: false, start: 70, end: 100, step: 5, repeats: 2 },
};
// A second loop without an end.
const loop3: PlayerLoop = { id: 'loop-3', start: { tick: 32 }, end: null, start_ms: 1000, end_ms: null, repeat: false, count_in: false, training: { on: false, start: 70, end: 100, step: 5, repeats: 2 } };
const withTempo = (): PlayerSnapshot => ({
  id: practice.id, title: 'T', sources: [{ key: 'backing:backing', label: 'Backing' }], source: 'backing:backing',
  duration_ms: 20000, sample_rate: 44100, peaks: [], mode: 'bpm',
  tempo: { beats_per_bar: 4, segments: [{ start_ms: 0, bpm: 120 }], multi: true }, speed_pct: 100,
  loops: [loop1, { ...loop1, id: 'loop-2' }, loop3], current_loop: 0,
  loop_message: null, transport: { ...idle, bpm: 120, tempo_index: 0 }, note: null, read_only: false, bpm_detect: null,
});
const timestamps = (): PlayerSnapshot => ({
  ...withTempo(), mode: 'time', tempo: { beats_per_bar: 4, segments: [], multi: false },
  loops: [{ ...loop1, start: { ms: 2500 }, start_ms: 2500, end: { ms: 6000 }, end_ms: 6000 }],
  transport: { ...idle },
});

let calls: [string, Record<string, unknown>][];
let channel: Channel<PlayerEvent>;
let snap: PlayerSnapshot;
let tempoHook: (() => Promise<PlayerSnapshot>) | null;
let detectHook: ((cmd: string) => PlayerSnapshot) | null;
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
  tempoHook = null;
  detectHook = null;
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'open_player') { channel = a.events as Channel<PlayerEvent>; return snap; }
    if ((cmd === 'player_detect_bpm' || cmd === 'player_cancel_detect') && detectHook) return detectHook(cmd);
    if (cmd === 'player_tempo' && tempoHook) return tempoHook();
    if (cmd === 'player_tempo' || cmd === 'player_loop') return snap;
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

describe('BPM', () => {
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

  it('timestamp mode disables the section toggles, -, + and Reset', async () => {
    await open(timestamps());
    expect(input('BPM').value).toBe('0');
    for (const n of ['Metronome in this section', 'Count-in into this section']) expect(btn('Sections', n).disabled).toBe(true);
    expect(screen.queryByRole('button', { name: 'On/Off' })).toBeNull();
    expect(screen.queryByRole('group', { name: 'Metronome' })).toBeNull();
    // The speed controls are in the Speed group, not in the BPM group.
    for (const n of ['Slower', 'Faster', 'Reset']) expect(group('BPM').queryByRole('button', { name: n })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Detect' })).toBeNull();
  });
});

describe('Speed group', () => {
  const box = () => input('Speed in percent');
  const actions = () => of('player_tempo').map((c) => c.action);

  it('- three times sends three speed-step -1; Reset is disabled at 100', async () => {
    await open(withTempo());
    expect(box().value).toBe('100');
    expect(btn('Speed', 'Reset').disabled).toBe(true);
    for (let i = 0; i < 3; i++) await fireEvent.click(btn('Speed', 'Slower'));
    await flush();
    expect(actions()).toEqual(Array(3).fill({ kind: 'speed-step', steps: -1 }));
  });

  it('typing 70 + Enter sends speed-set 70; 20 sends nothing and marks the box invalid', async () => {
    await open(withTempo());
    await type(box(), '70');
    expect(actions()).toEqual([{ kind: 'speed-set', percent: 70 }]);
    await type(box(), '20');
    expect(actions()).toHaveLength(1);
    expect(box().getAttribute('aria-invalid')).toBe('true');
    expect(box().title).not.toBe('');
    expect(screen.getByTestId('bpm-readout').textContent).toBe('25–200 %');
    await fireEvent.input(box(), { target: { value: '2' } });
    expect(box().getAttribute('aria-invalid')).toBeNull();
    await fireEvent.keyDown(box(), { key: 'Escape' });
    expect(box().value).toBe('100');
  });

  it('is disabled with the reason during a training run and when the player is not active', async () => {
    await open({ ...withTempo(), transport: { ...idle, training: { loop: 0, speed_pct: 70, repeat: 1, of: 2 } } });
    for (const n of ['Slower', 'Faster', 'Reset']) {
      expect(btn('Speed', n).disabled).toBe(true);
      expect(btn('Speed', n).title).toBe('Speed training is running; stop the loop to change the speed');
    }
    expect(box().disabled).toBe(true);
    expect(actions()).toHaveLength(0);
  });

  it('the readout shows the played BPM, empty for 0; the BPM box keeps the song BPM', async () => {
    await open(withTempo());
    expect(screen.getByTestId('bpm-readout').textContent).toBe('');
    expect(screen.getByTestId('bpm-readout').getAttribute('aria-label')).toBe('Playing BPM');
    channel.onmessage({ kind: 'transport', id: practice.id, ...idle, bpm: 120, played_bpm: 60, speed_pct: 50 });
    await waitFor(() => expect(screen.getByTestId('bpm-readout').textContent).toBe('60 BPM'));
    expect(input('BPM').value).toBe('120');
  });

  it('+ and - are disabled at the limits', async () => {
    await open({ ...withTempo(), speed_pct: 200 });
    expect(btn('Speed', 'Faster').disabled).toBe(true);
    expect(btn('Speed', 'Reset').disabled).toBe(false);
  });
});

describe('Training popover', () => {
  // jsdom has no layout, so floating-ui keeps the content `visibility: hidden` and the testing library gives hidden
  // elements no accessible name: find the popover's controls by their aria-label.
  const byLabel = (l: string) => document.querySelector(`[aria-label="${l}"]`);
  const trainBtn = () => btn('Loops', 'Speed training settings');
  const plans = () => of('player_loop').map((c) => c.action).filter((a) => (a as { kind: string }).kind === 'training');
  const logs = () => of('frontend_log').map((c) => String(c.message)).filter((m) => m.startsWith('player training-popover'));
  const closes = () => logs().filter((m) => m.includes(' close '));
  const dialog = () => byLabel('Speed training');
  const field = (n: string) => byLabel(n) as HTMLInputElement;
  const closeBtn = () => screen.getByText('Close');
  const openPopover = async () => {
    await fireEvent.click(trainBtn());
    await waitFor(() => expect(dialog()).not.toBeNull());
    await flush();
  };
  const on = (t: Partial<PlayerLoop['training']> = {}): PlayerSnapshot => {
    const s = withTempo();
    s.loops[0] = { ...loop1, training: { on: true, start: 60, end: 80, step: 10, repeats: 2, ...t } };
    return s;
  };

  it('is disabled without a complete loop and does not exist in the DOM until opened', async () => {
    await open({ ...withTempo(), current_loop: 2 });
    expect(trainBtn().disabled).toBe(true);
    expect(trainBtn().title).toBe('Set the loop end first');
    expect(trainBtn().textContent?.trim()).toBe('Train…');
    expect(dialog()).toBeNull();
  });

  it('is amber with the summary title when training is on', async () => {
    await open(withTempo());
    expect(trainBtn().title).toBe('Speed training settings');
    expect(trainBtn().className).not.toContain('bg-amber-500');
  });

  it('shows the plan in the title and in amber once training is on', async () => {
    await open(on());
    expect(trainBtn().className).toContain('bg-amber-500');
    expect(trainBtn().title).toBe('Speed training on: 60 → 80 %, +10 %, ×2');
  });

  it('opens with the focus on the switch; the switch sends the plan with on: true', async () => {
    await open(withTempo());
    await openPopover();
    const sw = byLabel('Training on') as HTMLElement;
    await waitFor(() => expect(document.activeElement).toBe(sw));
    expect(screen.getByText('Loop 1 of 3')).toBeTruthy();
    expect(screen.getByText('70 → 100 %, +5 % every 2 passes')).toBeTruthy();
    await fireEvent.click(sw);
    await flush();
    expect(plans()).toEqual([{ kind: 'training', on: true, start: 70, end: 100, step: 5, repeats: 2 }]);
    await waitFor(() => expect(logs().some((m) => m.startsWith('player training-popover open loop=0 rect='))).toBe(true));
  });

  it('typed values commit on Enter or on leaving; 0 in Step is refused with the message', async () => {
    await open(withTempo());
    await openPopover();
    await type(field('Start %'), '60');
    await type(field('End %'), '80');
    await type(field('Step %'), '10');
    await fireEvent.input(field('Repeats'), { target: { value: '2' } });
    await fireEvent.blur(field('Repeats'));
    await flush();
    expect(plans()).toEqual([
      { kind: 'training', on: false, start: 60, end: 100, step: 5, repeats: 2 },
      { kind: 'training', on: false, start: 70, end: 80, step: 5, repeats: 2 },
      { kind: 'training', on: false, start: 70, end: 100, step: 10, repeats: 2 },
      { kind: 'training', on: false, start: 70, end: 100, step: 5, repeats: 2 },
    ]);
    await type(field('Step %'), '0');
    expect(plans()).toHaveLength(4);
    expect(field('Step %').getAttribute('aria-invalid')).toBe('true');
    expect(screen.getByTestId('training-message').textContent).toBe('The step must be between 1 and 175 %');
    await fireEvent.input(field('Step %'), { target: { value: '5' } });
    expect(field('Step %').getAttribute('aria-invalid')).toBeNull();
    expect(screen.getByTestId('training-message').textContent).toBe('');
  });

  it('Escape closes it, drops an uncommitted edit and returns the focus to the button', async () => {
    await open(withTempo());
    await openPopover();
    await fireEvent.input(field('Start %'), { target: { value: '55' } });
    await fireEvent.keyDown(field('Start %'), { key: 'Escape' });
    await waitFor(() => expect(dialog()).toBeNull());
    expect(plans()).toEqual([]);
    expect(closes()).toEqual(['player training-popover close via=escape']);
    await waitFor(() => expect(document.activeElement).toBe(trainBtn()));
    await openPopover();
    expect(field('Start %').value).toBe('70');
  });

  it('a click outside closes it and sends a valid pending edit once', async () => {
    await open(withTempo());
    await openPopover();
    await fireEvent.input(field('End %'), { target: { value: '90' } });
    await new Promise((r) => setTimeout(r, 60)); // the layer ignores outside events in its first 20 ms
    await fireEvent.pointerDown(input('BPM'), { pointerType: 'mouse', button: 0, clientX: 500, clientY: 500 });
    await fireEvent.pointerUp(input('BPM'), { pointerType: 'mouse', button: 0, clientX: 500, clientY: 500 });
    await fireEvent.click(input('BPM'), { button: 0 });
    await waitFor(() => expect(dialog()).toBeNull());
    await flush();
    expect(plans()).toEqual([{ kind: 'training', on: false, start: 70, end: 90, step: 5, repeats: 2 }]);
    expect(closes()).toEqual(['player training-popover close via=outside']);
  });

  it('two dirty fields are sent as one merged plan on close, and quick commits do not overwrite each other', async () => {
    await open(withTempo());
    await openPopover();
    await fireEvent.input(field('Start %'), { target: { value: '50' } });
    await fireEvent.input(field('End %'), { target: { value: '90' } });
    await fireEvent.input(field('Step %'), { target: { value: '300' } }); // invalid: dropped, not sent
    await fireEvent.click(closeBtn());
    await waitFor(() => expect(dialog()).toBeNull());
    await flush();
    expect(plans()).toEqual([{ kind: 'training', on: false, start: 50, end: 90, step: 5, repeats: 2 }]);
    // two commits before the first answer: the second builds on the first
    calls.length = 0;
    await openPopover();
    await fireEvent.input(field('Start %'), { target: { value: '40' } });
    await fireEvent.input(field('Repeats'), { target: { value: '3' } });
    await fireEvent.keyDown(field('Start %'), { key: 'Enter' });
    await fireEvent.keyDown(field('Repeats'), { key: 'Enter' });
    await flush();
    expect(plans()).toEqual([
      { kind: 'training', on: false, start: 40, end: 100, step: 5, repeats: 2 },
      { kind: 'training', on: false, start: 40, end: 100, step: 5, repeats: 3 },
    ]);
  });

  it('qa: two edited fields are both saved when it closes by an outside click or by Tab from Close', async () => {
    await open(withTempo());
    await openPopover();
    await fireEvent.input(field('Start %'), { target: { value: '45' } });
    await fireEvent.input(field('Repeats'), { target: { value: '4' } });
    await new Promise((r) => setTimeout(r, 60));
    await fireEvent.pointerDown(input('BPM'), { pointerType: 'mouse', button: 0, clientX: 500, clientY: 500 });
    await fireEvent.pointerUp(input('BPM'), { pointerType: 'mouse', button: 0, clientX: 500, clientY: 500 });
    await fireEvent.click(input('BPM'), { button: 0 });
    await waitFor(() => expect(dialog()).toBeNull());
    await flush();
    expect(plans()).toEqual([{ kind: 'training', on: false, start: 45, end: 100, step: 5, repeats: 4 }]);
    calls.length = 0;
    await openPopover();
    await fireEvent.input(field('End %'), { target: { value: '95' } });
    await fireEvent.input(field('Step %'), { target: { value: '10' } });
    await fireEvent.keyDown(closeBtn(), { key: 'Tab' });
    await waitFor(() => expect(dialog()).toBeNull());
    await flush();
    expect(plans()).toEqual([{ kind: 'training', on: false, start: 70, end: 95, step: 10, repeats: 2 }]);
  });

  it('Close closes it, and Tab from Close leaves and closes it', async () => {
    await open(withTempo());
    await openPopover();
    await fireEvent.click(closeBtn());
    await waitFor(() => expect(dialog()).toBeNull());
    await openPopover();
    await fireEvent.keyDown(closeBtn(), { key: 'Tab' });
    await waitFor(() => expect(dialog()).toBeNull());
    expect(closes()).toEqual(['player training-popover close via=button', 'player training-popover close via=tab']);
  });

  it('the status line shows the progress, an action error replaces it, and its height stays', async () => {
    await open(withTempo());
    const status = screen.getByTestId('loop-status');
    expect(status.className).toContain('min-h-5');
    channel.onmessage({ kind: 'transport', id: practice.id, ...idle, speed_pct: 70, training: { loop: 0, speed_pct: 70, repeat: 2, of: 4 } });
    await waitFor(() => expect(status.textContent).toBe('Training 70 % · 2/4'));
    expect(status.className).toContain('min-h-5');
    pl.actionError = 'Boom';
    await flush();
    expect(status.textContent).toBe('Boom');
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

describe('player.json problems', () => {
  it('shows the note, the save error, and clears the save error after a later snapshot', async () => {
    await open({ ...withTempo(), note: 'player.json is newer; read-only' });
    expect(screen.getByTestId('player-note').textContent).toContain('read-only');
    expect(screen.queryByTestId('player-state-error')).toBeNull();
    channel.onmessage({ kind: 'state-error', id: practice.id, message: 'Could not save' } as PlayerEvent);
    await flush();
    expect(screen.getByTestId('player-state-error').textContent).toContain('Could not save');
    await type(input('BPM'), '90');
    expect(screen.queryByTestId('player-state-error')).toBeNull();
  });

  it('shows the note and the save error together, each in full', async () => {
    await open({ ...withTempo(), note: 'player.json is newer; read-only' });
    channel.onmessage({ kind: 'state-error', id: practice.id, message: 'Could not save' } as PlayerEvent);
    await flush();
    expect(screen.getByTestId('player-note').textContent).toContain('read-only');
    expect(screen.getByTestId('player-state-error').textContent).toContain('Could not save');
    expect(screen.getByTestId('placeholder').className).toContain('min-h-fit');
  });

  it('shows Loop On unpressed while it is disabled on a loop without an end', async () => {
    await open({ ...withTempo(), current_loop: 0, loops: [{ ...loop3, repeat: true }] });
    const b = screen.getByRole('button', { name: 'Loop On' });
    expect(b.getAttribute('aria-pressed')).toBe('false');
  });
});

describe('Sections group', () => {
  const three = (bpms = [120, 93, 160], idx = 1): PlayerSnapshot => ({
    ...withTempo(),
    tempo: { beats_per_bar: 4, multi: true, segments: bpms.map((bpm, i) => ({ start_ms: i * 6000, bpm, on_beat: i > 0 })) },
    transport: { ...idle, bpm: bpms[idx], tempo_index: idx },
  });
  const wf = () => screen.getByTestId('player-waveform');

  it('shows the target and enables Add, Remove, x2 and /2', async () => {
    await open(three());
    expect(screen.getByTestId('section-status').textContent).toBe('Section 2 of 3 · 93 BPM');
    for (const n of ['Add a tempo change at the playhead', 'Remove the tempo change', "Double the section's BPM", "Halve the section's BPM"]) {
      expect(btn('Sections', n).disabled).toBe(false);
    }
  });

  it('BPM 0 disables the group with a reason', async () => {
    await open(timestamps());
    expect(screen.getByTestId('section-status').textContent).toBe('No tempo');
    const add = btn('Sections', 'Add a tempo change at the playhead');
    expect(add.disabled).toBe(true);
    expect(add.title).toBe('Type a BPM first');
    expect(btn('Sections', 'Remove the tempo change').disabled).toBe(true);
    expect(screen.getByTestId('section-message').textContent).toBe('Type a BPM first');
  });

  it('section 1 has no Remove, by button or by key', async () => {
    await open(three([120, 93, 160], 0));
    const rm = btn('Sections', 'Remove the tempo change');
    expect(rm.disabled).toBe(true);
    expect(rm.title).not.toBe('');
    wf().focus();
    await fireEvent.keyDown(wf(), { key: 'Delete' });
    await fireEvent.keyDown(wf(), { key: 'Backspace' });
    await flush();
    expect(of('player_tempo')).toHaveLength(0);
  });

  it('x2 is disabled at 160 BPM and /2 at 25 BPM, each with its reason; 93 allows both', async () => {
    await open(three([120, 93, 160], 2));
    const x2 = btn('Sections', "Double the section's BPM");
    expect(x2.disabled).toBe(true);
    expect(x2.title).toMatch(/300/);
    expect(btn('Sections', "Halve the section's BPM").disabled).toBe(false);
    pl.section = 1;
    await flush();
    expect(btn('Sections', "Double the section's BPM").disabled).toBe(false);
    expect(btn('Sections', "Halve the section's BPM").disabled).toBe(false);
    pl.section = 2;
    pl.snap!.tempo.segments[2].bpm = 25;
    await flush();
    const half = btn('Sections', "Halve the section's BPM");
    expect(half.disabled).toBe(true);
    expect(half.title).toMatch(/20/);
  });

  it('the buttons send the matching player_tempo actions', async () => {
    await open(three());
    await fireEvent.click(btn('Sections', "Double the section's BPM"));
    await fireEvent.click(btn('Sections', "Halve the section's BPM"));
    await fireEvent.click(btn('Sections', 'Remove the tempo change'));
    await flush();
    expect(of('player_tempo').map((c) => c.action)).toEqual([
      { kind: 'scale-section', index: 1, at: 6000, factor: 'double' },
      { kind: 'scale-section', index: 1, at: 6000, factor: 'halve' },
      { kind: 'remove-change', index: 1, at: 6000 },
    ]);
  });

  describe('Find start', () => {
    const find = () => btn('Sections', 'Find where the music starts and put the first beat there');
    const found = (): PlayerSnapshot => {
      const t = three([120, 93, 160], 0);
      t.tempo.segments[0] = { ...t.tempo.segments[0], start_ms: 1200, found_ms: 1200 };
      return t;
    };

    it('is disabled without a tempo', async () => {
      await open(timestamps());
      expect(find().disabled).toBe(true);
      expect(find().title).toBe('Type a BPM first');
    });

    it('is disabled outside section 1 with a reason', async () => {
      await open(three());
      expect(find().disabled).toBe(true);
      expect(find().title).toBe('Find start works in the first section');
    });

    it('sends find-start, is disabled while it runs and then shows the new start', async () => {
      await open(three([120, 93, 160], 0));
      expect(find().disabled).toBe(false);
      let done!: (s: PlayerSnapshot) => void;
      tempoHook = () => new Promise((res) => { done = res; });
      await fireEvent.click(find());
      await flush();
      expect(of('player_tempo').map((c) => c.action)).toEqual([{ kind: 'find-start' }]);
      expect(find().disabled).toBe(true);
      expect(find().title).toBe('Finding the start...');
      expect(screen.getByTestId('section-message').textContent).toBe('Finding the start...');
      done(found());
      await flush();
      expect(find().disabled).toBe(false);
      expect(screen.getByTestId('section-message').textContent).toBe('First beat at 0:01.200');
      expect(of('frontend_log').map((c) => c.message)).toContain('player first-beat start=1200');
    });

    it('shows a refusal in the message line', async () => {
      await open(three([120, 93, 160], 0));
      tempoHook = () => Promise.reject('Nothing but noise: no start found');
      await fireEvent.click(find());
      await flush();
      expect(screen.getByTestId('section-message').textContent).toBe('Nothing but noise: no start found');
      expect(find().disabled).toBe(false);
    });
  });

  describe('Start offset row and Prev / Next', () => {
    const S = (idx = 0, start0 = 1200): PlayerSnapshot => ({
      ...withTempo(),
      duration_ms: 5200,
      tempo: {
        beats_per_bar: 4, multi: true,
        segments: [
          { start_ms: start0, bpm: 120, found_ms: 1200 },
          { start_ms: 3000, bpm: 120, detected_ms: 3000 },
          { start_ms: 4200, bpm: 120 },
        ],
      },
      transport: { ...idle, bpm: 120, tempo_index: idx },
    });
    const field = () => input('Start');
    const time = () => screen.getByTestId('section-offset-time').textContent;
    const prev = () => btn('Sections', 'Go to the start of the previous section');
    const next = () => btn('Sections', 'Go to the start of the next section');
    const actions = () => of('player_tempo').map((c) => c.action);
    const ev = (tempo_index: number) => channel.onmessage({ kind: 'transport', id: practice.id, ...idle, bpm: 120, tempo_index });

    it('shows 0 and +0:00.000 for section 1', async () => {
      await open(S());
      expect(field().value).toBe('0');
      expect(time()).toBe('+0:00.000');
    });

    it('+10 and Enter sends the move; the answer shows +10 and +0:00.078 and the field loses the focus', async () => {
      await open(S());
      field().focus();
      snap = S(0, 1278.125);
      await type(field(), '+10');
      expect(actions()).toEqual([{ kind: 'move-change', index: 0, at: 1200, steps: 10 }]);
      expect(field().value).toBe('+10');
      expect(time()).toBe('+0:00.078');
      expect(document.activeElement).not.toBe(field());
    });

    it('text x and Enter shows the refusal, keeps the text and the focus; blur restores the value', async () => {
      await open(S());
      field().focus();
      await type(field(), 'x');
      const msg = screen.getByTestId('section-message');
      expect(msg.textContent).toBe('Type a whole number of 1/64 beats, like +10 or -8');
      expect(msg.getAttribute('role')).toBe('alert');
      expect(field().value).toBe('x');
      expect(field().getAttribute('aria-invalid')).toBe('true');
      expect(of('player_tempo')).toHaveLength(0);
      await fireEvent.blur(field());
      await flush();
      expect(field().value).toBe('0');
      expect(of('player_tempo')).toHaveLength(0);
      expect(field().getAttribute('aria-invalid')).toBeNull();
      expect(msg.textContent).toBe('Type a whole number of 1/64 beats, like +10 or -8');
    });

    it('follows the playhead into another section', async () => {
      await open(S());
      ev(1);
      await flush();
      expect(screen.getByTestId('section-status').textContent).toContain('Section 2 of 3');
      expect(field().value).toBe('0');
      expect(time()).toBe('+0:00.000');
    });

    it('follows a key move and Find start', async () => {
      await open(S());
      snap = S(0, 1223.4375);
      const { moveChange } = await import('$lib/player-state.svelte');
      moveChange(0, 3);
      await flush();
      expect(field().value).toBe('+3');
      snap = S(0, 1200);
      await fireEvent.click(btn('Sections', 'Find where the music starts and put the first beat there'));
      await flush();
      expect(field().value).toBe('0');
    });

    it('section 2: Find start disabled, the field enabled; -8 sends index 1', async () => {
      await open(S(1));
      const f = btn('Sections', 'Find where the music starts and put the first beat there');
      expect(f.disabled).toBe(true);
      expect(f.title).toBe('Find start works in the first section');
      expect(field().disabled).toBe(false);
      field().focus();
      await type(field(), '-8');
      expect(actions()).toEqual([{ kind: 'move-change', index: 1, at: 3000, steps: -8 }]);
    });

    describe('Detect BPM button', () => {
      const detect = () => screen.getByTestId('detect-bpm') as HTMLButtonElement;
      const msg = () => screen.getByTestId('section-message');
      const view = { serial: 1, index: 2, at: 4200, from_ms: 10500, to_ms: 17700, source: 'backing:backing', phase: 'checking' as const };

      it('follows Find start in the Start row, is enabled with a tempo and shows the unit /64', async () => {
        await open(S(1));
        const find = btn('Sections', 'Find where the music starts and put the first beat there');
        expect(find.nextElementSibling).toBe(detect());
        expect(detect().textContent).toBe('Detect BPM');
        expect(detect().disabled).toBe(false);
        expect(detect().title).toBe('');
        expect(screen.getByText('/64').title).toBe('1/64 beat');
      });

      it('is disabled with its reason without a tempo', async () => {
        await open(timestamps());
        expect(detect().disabled).toBe(true);
        expect(detect().title).toBe('Type a BPM first');
      });

      it('is disabled with the read-only reason when the settings are read-only', async () => {
        await open({ ...S(0), read_only: true });
        expect(detect().disabled).toBe(true);
        expect(detect().title).toBe("This track's Player settings are read-only");
      });

      it('acts on the shown section, not the one at the playhead', async () => {
        await open(S(0));
        selectSection(2);
        await flush();
        expect(detect().disabled).toBe(false);
        detectHook = () => ({ ...S(0), bpm_detect: view });
        await fireEvent.click(detect());
        await flush();
        expect(of('player_detect_bpm')).toEqual([{ id: practice.id, index: 2, at: 4200 }]);
      });

      it('while running reads Cancel and shows the running text; Cancel sends the command and restores the button', async () => {
        await open(S(2));
        detectHook = (cmd) => ({ ...S(2), bpm_detect: cmd === 'player_detect_bpm' ? view : null });
        await fireEvent.click(detect());
        await flush();
        expect(detect().textContent).toBe('Cancel');
        expect(detect().disabled).toBe(false);
        expect(msg().textContent).toBe('Detecting the BPM from 0:10.5 – 0:17.7...');
        await fireEvent.click(detect());
        await flush();
        expect(of('player_cancel_detect')).toEqual([{ id: practice.id }]);
        expect(detect().textContent).toBe('Detect BPM');
        expect(detect().disabled).toBe(false);
        expect(msg().textContent).toBe('BPM detection cancelled; the section is unchanged');
      });

      it('an applied result shows the status text and the new BPM; a failed one shows an alert', async () => {
        await open(S(2));
        detectHook = () => ({ ...S(2), bpm_detect: view });
        await fireEvent.click(detect());
        await flush();
        const after = S(2);
        after.tempo.segments[2].bpm = 93.4;
        const r = { serial: 1, index: 2, outcome: 'applied' as const, old_bpm: 120, new_bpm: 93.4, from_ms: 10500, to_ms: 17700, hint: null, message: null };
        channel.onmessage({ kind: 'bpm-detected', id: practice.id, result: r, snapshot: after });
        await flush();
        expect(msg().textContent).toBe('BPM 120.0 → 93.4, detected from 0:10.5 – 0:17.7');
        expect(screen.getByTestId('section-status').textContent).toBe('Section 3 of 3 · 93.4 BPM');
        detectHook = () => ({ ...S(2), bpm_detect: { ...view, serial: 2 } });
        await fireEvent.click(detect());
        await flush();
        channel.onmessage({ kind: 'bpm-detected', id: practice.id, result: { ...r, serial: 2, outcome: 'failed', new_bpm: null, message: 'No beats found' }, snapshot: null });
        await flush();
        expect(msg().textContent).toBe('No beats found');
        expect(msg().getAttribute('role')).toBe('alert');
        expect(detect().textContent).toBe('Detect BPM');
      });
    });

    it('Prev is disabled in section 1 and Next in section 3; both work in section 2', async () => {
      await open(S(0));
      expect(prev().disabled).toBe(true);
      expect(prev().title).toBe('This is the first section');
      expect(next().disabled).toBe(false);
      ev(2);
      await flush();
      expect(next().disabled).toBe(true);
      expect(next().title).toBe('This is the last section');
      ev(1);
      await flush();
      expect(prev().disabled).toBe(false);
      expect(next().disabled).toBe(false);
      await fireEvent.click(next());
      await fireEvent.click(prev());
      await flush();
      expect(actions()).toEqual([
        { kind: 'jump-section', index: 2, at: 4200, edge: 'start' },
        { kind: 'jump-section', index: 1, at: 3000, edge: 'start' },
      ]);
    });

    const legacy = (idx = 0): PlayerSnapshot => {
      const s = S(idx);
      s.tempo.segments[0] = { start_ms: 0, bpm: 120 };
      return s;
    };

    it('qa: a legacy track (no found_ms) counts from its start; Prev into section 1 jumps to 0:00', async () => {
      await open(legacy(1));
      await fireEvent.click(prev());
      await flush();
      expect(actions()).toEqual([{ kind: 'jump-section', index: 0, at: 0, edge: 'start' }]);
      expect(pl.section).toBe(0);
      expect(field().value).toBe('0');
      expect(time()).toBe('+0:00.000');
      expect(prev().disabled).toBe(true);
      field().focus();
      await type(field(), '+10');
      expect(actions()[1]).toEqual({ kind: 'move-change', index: 0, at: 0, steps: 10 });
      expect(field().title).toBe('1/64 beats from the found start');
    });

    it('qa: a typed section (no detected_ms) counts from the previous start and clamps to one beat after it', async () => {
      await open(S(2));
      expect(field().title).toBe("1/64 beats from the previous section's start");
      expect(field().value).toBe('+154');
      expect(time()).toBe('+0:01.200');
      field().focus();
      await type(field(), '-9999');
      const a = actions().at(-1) as { kind: string; index: number; at: number; steps: number };
      expect(a).toMatchObject({ kind: 'move-change', index: 2, at: 4200 });
      expect(a.steps).toBe(64); // 3000 + one beat of 500 ms = 64 steps of 7.8125 ms
    });

    it('qa: the detected section field is titled from the detected position', async () => {
      await open(S(1));
      expect(field().title).toBe('1/64 beats from the detected position');
    });

    it('qa: typing while the playhead crosses a boundary still edits the section that had the focus', async () => {
      await open(S(0));
      field().focus();
      await fireEvent.input(field(), { target: { value: '+5' } });
      ev(1);
      await flush();
      expect(field().value).toBe('+5'); // the typed text is kept, the pane keeps its captured section
      await fireEvent.keyDown(field(), { key: 'Enter' });
      await flush();
      expect(actions()).toEqual([{ kind: 'move-change', index: 0, at: 1200, steps: 5 }]);
    });

    it('qa: a typed value equal to the applied one sends nothing; a limit hit shows the applied value', async () => {
      await open(S(0));
      field().focus();
      await type(field(), '0');
      expect(actions()).toEqual([]);
      field().focus();
      snap = S(0, 0);
      await type(field(), '-9999');
      expect(actions()).toEqual([{ kind: 'move-change', index: 0, at: 1200, steps: -154 }]);
      expect(field().value).toBe('-154');
      expect(time()).toBe('-0:01.200');
    });

    it('qa: whole-number edge texts: 1.5, 1e3, empty and + are refused; a unicode minus is accepted', async () => {
      await open(S(0));
      for (const t of ['1.5', '1e3', '', '+', ' ']) {
        field().focus();
        await type(field(), t);
        expect(screen.getByTestId('section-message').textContent).toBe('Type a whole number of 1/64 beats, like +10 or -8');
        await fireEvent.keyDown(field(), { key: 'Escape' });
        await flush();
      }
      expect(actions()).toEqual([]);
      field().focus();
      await type(field(), '\u22128');
      expect(actions()).toEqual([{ kind: 'move-change', index: 0, at: 1200, steps: -8 }]);
    });

    it('qa: a read-only (newer schema) player.json has no tempo: field, Prev, Next and Find start are disabled and send nothing', async () => {
      const ro = S(0);
      ro.tempo = { beats_per_bar: 4, multi: false, segments: [] };
      ro.transport = { ...idle };
      ro.note = 'player.json was written by a newer version; the player settings will not be saved';
      await open(ro);
      for (const el of [field(), prev(), next(), btn('Sections', 'Find where the music starts and put the first beat there')]) {
        expect(el.disabled).toBe(true);
        await fireEvent.click(el);
      }
      await flush();
      expect(of('player_tempo')).toHaveLength(0);
    });

    it('qa: a refusal from the backend (newer schema) shows in the message line and leaves the field on the stored value', async () => {
      await open(S(0));
      tempoHook = () => Promise.reject('player.json was written by a newer version; the tempo was not saved');
      field().focus();
      await type(field(), '+10');
      expect(screen.getByTestId('section-message').textContent).toBe('player.json was written by a newer version; the tempo was not saved');
      expect(field().value).toBe('0');
      expect(time()).toBe('+0:00.000');
      await fireEvent.click(next());
      await flush();
      expect(pl.snap!.tempo.segments[0].start_ms).toBe(1200);
    });

    it('without a tempo the field, Prev, Next and Find start are disabled with the reason', async () => {
      await open(timestamps());
      for (const el of [field(), prev(), next(), btn('Sections', 'Find where the music starts and put the first beat there')]) {
        expect(el.disabled).toBe(true);
        expect(el.title).toBe('Type a BPM first');
      }
    });
  });

  it('Add captures the playhead for the overlay (the overlay itself is tested with the waveform)', async () => {
    await open(three());
    pl.transport.position_ms = 7300;
    await fireEvent.click(btn('Sections', 'Add a tempo change at the playhead'));
    expect(pl.bpmEdit).toEqual({ addAt: 7300 });
  });

  describe('metronome, name and jumps', () => {
    const meta = (idx = 1): PlayerSnapshot => {
      const t = three([120, 93, 160], idx);
      t.tempo.segments[1] = { ...t.tempo.segments[1], name: 'Verse', metronome: false, count_in: true };
      return t;
    };
    const tempoActions = () => of('player_tempo').map((c) => c.action);

    it('the toggles show the target meta; section 1 has Count In', async () => {
      await open(meta());
      expect(btn('Sections', 'Metronome in this section').getAttribute('aria-pressed')).toBe('false');
      expect(btn('Sections', 'Count-in into this section').getAttribute('aria-pressed')).toBe('true');
      pl.section = 0;
      await flush();
      expect(btn('Sections', 'Metronome in this section').getAttribute('aria-pressed')).toBe('true');
      expect(btn('Sections', 'Count-in into this section').disabled).toBe(false);
    });

    it('clicks send section-flags', async () => {
      await open(meta());
      await fireEvent.click(btn('Sections', 'Metronome in this section'));
      await fireEvent.click(btn('Sections', 'Count-in into this section'));
      await flush();
      expect(tempoActions()).toEqual([
        { kind: 'section-flags', index: 1, at: 6000, metronome: true, count_in: true },
        { kind: 'section-flags', index: 1, at: 6000, metronome: false, count_in: false },
      ]);
    });

    it('Start and End send jump-section', async () => {
      await open(meta());
      await fireEvent.click(btn('Sections', 'Jump to the start of the section'));
      await fireEvent.click(btn('Sections', 'Jump to the end of the section'));
      await flush();
      expect(tempoActions()).toEqual([
        { kind: 'jump-section', index: 1, at: 6000, edge: 'start' },
        { kind: 'jump-section', index: 1, at: 6000, edge: 'end' },
      ]);
    });

    it('the name box shows the name; Enter commits, Escape restores, unchanged sends nothing', async () => {
      await open(meta());
      const box = input('Section name');
      expect(box.value).toBe('Verse');
      expect(box.placeholder).toBe('Name this section');
      expect(box.maxLength).toBe(40);
      box.focus();
      await fireEvent.input(box, { target: { value: ' Chorus ' } });
      await fireEvent.keyDown(box, { key: 'Enter' });
      await flush();
      expect(tempoActions()).toEqual([{ kind: 'section-name', index: 1, at: 6000, name: 'Chorus' }]);
      box.focus();
      await fireEvent.input(box, { target: { value: 'Other' } });
      await fireEvent.keyDown(box, { key: 'Escape' });
      await flush();
      expect(tempoActions()).toHaveLength(1);
      expect(box.value).toBe('Verse');
      box.focus();
      await fireEvent.blur(box);
      await flush();
      expect(tempoActions()).toHaveLength(1);
    });

    it('blur commits, and the box keeps its section when the playhead moves on', async () => {
      await open(meta());
      const box = input('Section name');
      box.focus();
      await fireEvent.input(box, { target: { value: 'Bridge' } });
      channel.onmessage({ kind: 'transport', id: practice.id, ...idle, bpm: 160, tempo_index: 2 });
      await flush();
      expect(screen.getByTestId('section-status').textContent).toBe('Section 2 of 3 · 93 BPM');
      await fireEvent.blur(box);
      await flush();
      expect(tempoActions()).toEqual([{ kind: 'section-name', index: 1, at: 6000, name: 'Bridge' }]);
    });

    it('section 1 Count In is enabled and sends section-flags index=0 count_in=true', async () => {
      await open(meta());
      pl.section = 0;
      await flush();
      const ci = btn('Sections', 'Count-in into this section');
      expect(ci.disabled).toBe(false);
      expect(ci.title).toBe('');
      await fireEvent.click(ci);
      await flush();
      expect(tempoActions()).toEqual([{ kind: 'section-flags', index: 0, at: 0, metronome: true, count_in: true }]);
      expect(screen.queryByRole('button', { name: 'On/Off' })).toBeNull();
      expect(screen.queryByRole('group', { name: 'Metronome' })).toBeNull();
    });
  });

  it('the message slot keeps its height when empty or filled', async () => {
    await open(three());
    const slot = screen.getByTestId('section-message');
    expect(slot.className).toContain('min-h-5');
    expect(slot.className).toContain('truncate');
    pl.sectionError = 'Too close to the neighbour';
    await flush();
    expect(slot.textContent).toBe('Too close to the neighbour');
    expect(slot.title).toBe('Too close to the neighbour');
    expect(slot.className).toContain('min-h-5');
  });
});
