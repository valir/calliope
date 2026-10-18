// Independent acceptance tests for specs/gui-play-backing-track.md, increment A (plan section 7),
// through the rendered components with mocked IPC (what a user sees and presses).
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import type { Channel } from '@tauri-apps/api/core';
import PlayerView from './views/PlayerView.svelte';
import App from './App.svelte';
import { pl, resetPlayer, selectSection } from '$lib/player-state.svelte';
import { lib, resetLibrary, selectTrack } from '$lib/library-state.svelte';
import { ui } from '$lib/app-state.svelte';
import { FIXTURE_TRACKS, PLAYER_FIXTURE_TRACKS } from '$lib/fixture-tracks';
import { parseBpm, formatBpm } from '$lib/bpm-format';
import { formatBar, parseBar, formatLoopTime, parseLoopTime } from '$lib/bar-format';
import type { PlayerEvent, PlayerLoop, PlayerSnapshot, PlayerTransport } from '$lib/ipc';

const [practice] = PLAYER_FIXTURE_TRACKS;
const idle: PlayerTransport = {
  playing: false, counting_in: false, resume_pending: false, position_ms: 0, loop_playing: false, bpm: 0, tempo_index: null,
};
const complete: PlayerLoop = {
  id: 'loop-1', start: { tick: 64 }, end: { tick: 192 }, start_ms: 2000, end_ms: 10000, repeat: true, count_in: false,
};
const open_: PlayerLoop = { id: 'loop-3', start: { tick: 512 }, end: null, start_ms: 18500, end_ms: null, repeat: true, count_in: false };
const base = (): PlayerSnapshot => ({
  id: practice.id, title: 'T', sources: [{ key: 'backing:backing', label: 'Backing' }, { key: 'stem:guitar', label: 'Guitar' }],
  source: 'backing:backing', duration_ms: 20000, sample_rate: 22050, peaks: new Array(64).fill(0.5), mode: 'time',
  tempo: { beats_per_bar: 4, segments: [], multi: false }, speed: 1,
  loops: [], current_loop: null, loop_message: null, transport: { ...idle }, can_change_speed: false, note: null,
});
const bpmSnap = (): PlayerSnapshot => ({
  ...base(), mode: 'bpm', tempo: { beats_per_bar: 4, segments: [{ start_ms: 500, bpm: 120 }], multi: false },
  loops: [complete, open_], current_loop: 0, transport: { ...idle, bpm: 120, tempo_index: 0 },
});

let calls: [string, Record<string, unknown>][];
let snap: PlayerSnapshot;
let channel: Channel<PlayerEvent> | null;
const flush = () => new Promise((r) => setTimeout(r, 0));
const of = (cmd: string) => calls.filter((c) => c[0] === cmd).map((c) => c[1]);
const group = (name: string) => within(screen.getByRole('group', { name }));
const btn = (g: string, name: string) => group(g).getByRole('button', { name }) as HTMLButtonElement;
const input = (name: string) => screen.getByRole('textbox', { name }) as HTMLInputElement;

async function openTrack(s: PlayerSnapshot) {
  snap = s;
  await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
  selectTrack(practice.id);
  await waitFor(() => expect(pl.status).toBe('active'));
  await flush();
}

beforeEach(() => {
  calls = [];
  channel = null;
  resetPlayer();
  resetLibrary();
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'open_player') { channel = a.events as Channel<PlayerEvent>; return snap; }
    if (cmd === 'player_tempo' || cmd === 'player_loop' || cmd === 'player_select_source') return snap;
    if (cmd === 'player_leave') return { ...idle };
    if (cmd.startsWith('player_')) return { ...idle };
    if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/r', tracks: [practice, ...FIXTURE_TRACKS.slice(0, 2)], problems: [] };
    return null;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
  ui.view = 'library';
});

describe('AC2-4, 9: pane structure and the inactive state', () => {
  it('without a selected track the pane is inactive and its controls are disabled', async () => {
    snap = base();
    render(PlayerView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    expect(pl.status).toBe('inactive');
    expect(document.body.textContent).toMatch(/Select a track/);
    expect(btn('Playback', 'Play').disabled).toBe(true);
    expect(btn('Playback', 'Rew').disabled).toBe(true);
    expect(btn('Playback', 'FF').disabled).toBe(true);
    expect(input('BPM').disabled).toBe(true);
    expect(btn('Loops', 'Add loop').disabled).toBe(true);
    expect(input('bar1').disabled).toBe(true);
  });

  it('a track with audio shows source dropdown, tablature placeholder, waveform and the four sections', async () => {
    snap = base();
    render(PlayerView);
    await openTrack(snap);
    expect(screen.getByRole('button', { name: 'Audio source' }).textContent).toContain('Backing');
    expect(screen.getByTestId('placeholder')).toBeTruthy();
    for (const g of ['BPM', 'Sections', 'Playback', 'Loops']) expect(screen.getByRole('group', { name: g })).toBeTruthy();
    expect(document.querySelector('canvas')).toBeTruthy();
    expect(btn('Playback', 'Play').disabled).toBe(false);
  });

  it('lays the controls out in two rows: Sections (BPM inside) and Loops, then Playback', async () => {
    snap = base();
    render(PlayerView);
    await openTrack(snap);
    const controls = screen.getByTestId('player-controls');
    const rows = Array.from(controls.children) as HTMLElement[];
    expect(rows.length).toBe(2);
    const names = (el: HTMLElement) => Array.from(el.children).map((c) => c.getAttribute('aria-label'));
    expect(names(rows[0])).toEqual(['Sections', 'Loops']);
    expect(rows[1].getAttribute('aria-label')).toBe('Playback');
    const sections = screen.getByRole('group', { name: 'Sections' });
    const bpm = within(sections).getByRole('group', { name: 'BPM' });
    const status = screen.getByTestId('section-status');
    expect(bpm.compareDocumentPosition(status) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(group('Loops').getByText('Loops')).toBeTruthy();
    const playback = group('Playback');
    for (const n of ['Rew', 'Play', 'FF']) expect(playback.getByRole('button', { name: n })).toBeTruthy();
    expect(playback.getByRole('textbox')).toBeTruthy();
    expect(playback.getByRole('slider', { name: 'Position' })).toBeTruthy();
  });
});

describe('AC13: track without tempo', () => {
  it('BPM 0, V unchecked, timestamps in loop boxes, tempo features disabled', async () => {
    snap = { ...base(), loops: [{ id: 'loop-1', start: { ms: 2500 }, end: { ms: 6000 }, start_ms: 2500, end_ms: 6000, repeat: true, count_in: false }], current_loop: 0 };
    render(PlayerView);
    await openTrack(snap);
    expect(input('BPM').value).toBe('0');
    expect((screen.getByRole('checkbox', { name: 'V' }) as HTMLInputElement).checked).toBe(false);
    expect(input('bar1').value).toMatch(/^0:02\.5/);
    expect(input('bar2').value).toMatch(/^0:06\.0/);
    for (const n of ['Metronome in this section', 'Count-in into this section']) {
      expect(btn('Sections', n).disabled).toBe(true);
      expect(btn('Sections', n).title).toBe('Type a BPM first');
    }
    expect(screen.queryByRole('group', { name: 'Metronome' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Slower' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Faster' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Reset' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Detect' })).toBeNull();
  });

  it('on a track with a tempo (increment A) -, +, Reset stay hidden', async () => {
    snap = bpmSnap();
    render(PlayerView);
    await openTrack(snap);
    expect(input('BPM').value).toBe('120');
    for (const n of ['Slower', 'Faster', 'Reset']) expect(screen.queryByRole('button', { name: n }), n).toBeNull();
  });
});

describe('BPM field edge values', () => {
  it('parseBpm accepts only 0 and 20..300 with at most one decimal', () => {
    const ok: [string, number][] = [['0', 0], ['0.0', 0], ['20', 20], ['300', 300], ['99.9', 99.9], [' 120 ', 120], ['120.0', 120], ['020', 20], ['00', 0]];
    for (const [t, v] of ok) expect(parseBpm(t), t).toBe(v);
    const bad = ['', ' ', '.', '.5', '120.', '19.9', '300.1', '301', '-1', '-0', '+120', '1e2', '0x10', '120.55', '120,5', 'abc', 'NaN', 'Infinity', '٣٠٠', '１２０', '120 bpm', '1 20', '99999999999999999999', '120\n130'];
    for (const t of bad) expect(parseBpm(t), JSON.stringify(t)).toBeNull();
    expect(formatBpm(120)).toBe('120');
    expect(formatBpm(99.9)).toBe('99.9');
    expect(formatBpm(0)).toBe('0');
  });

  it('typing a bad value marks it invalid and sends nothing; Escape restores; Enter on a good value sends', async () => {
    snap = base();
    render(PlayerView);
    await openTrack(snap);
    const f = input('BPM');
    for (const bad of ['abc', '5', '301', '-3', '1e3', '12.55']) {
      await fireEvent.input(f, { target: { value: bad } });
      await fireEvent.keyDown(f, { key: 'Enter' });
      await flush();
      expect(f.getAttribute('aria-invalid'), bad).toBe('true');
      expect(screen.getByTestId('bpm-note').textContent).toMatch(/20 and 300/);
    }
    expect(of('player_tempo')).toHaveLength(0);
    await fireEvent.keyDown(f, { key: 'Escape' });
    await tick();
    expect(f.value).toBe('0');
    await fireEvent.input(f, { target: { value: '120' } });
    await fireEvent.keyDown(f, { key: 'Enter' });
    await flush();
    expect(of('player_tempo')).toEqual([expect.objectContaining({ action: { kind: 'set-bpm', bpm: 120 } })]);
  });
});

describe('AC22, 30, 31: loops without an end', () => {
  it('+ sends add; the box shows number/count; an open loop disables Play, Loop On, Count-In and empties bar2', async () => {
    snap = { ...base(), mode: 'time', loops: [{ ...open_, start: { ms: 3000 }, start_ms: 3000 }], current_loop: 0 };
    render(PlayerView);
    await openTrack(snap);
    expect((screen.getByTestId('loop-box') as HTMLInputElement).value).toBe('1/1');
    expect(input('bar2').value).toBe('');
    expect(input('bar1').value).not.toBe('');
    expect(btn('Loops', 'Play').disabled).toBe(true);
    expect(btn('Loops', 'Loop On').disabled).toBe(true);
    expect(btn('Loops', 'Count-In').disabled).toBe(true);
    const slider = screen.getByTestId('loop-slider');
    expect(slider.getAttribute('aria-valuenow')).toBeNull();
    expect(slider.getAttribute('aria-readonly')).toBe('true');
    await fireEvent.click(btn('Loops', 'Add loop'));
    await flush();
    expect(of('player_loop').pop()).toEqual(expect.objectContaining({ action: { kind: 'add' } }));
    await fireEvent.click(btn('Loops', 'Remove loop'));
    await flush();
    expect(of('player_loop').pop()).toEqual(expect.objectContaining({ action: { kind: 'remove' } }));
  });

  it('a refused end mark shows "The end must be after the start"', async () => {
    snap = { ...base(), loops: [{ ...open_, start: { ms: 3000 }, start_ms: 3000 }], current_loop: 0 };
    render(PlayerView);
    await openTrack(snap);
    snap = { ...snap, loop_message: 'The end must be after the start' };
    await fireEvent.click(btn('Loops', 'Mark end'));
    await flush();
    await waitFor(() => expect(screen.getByTestId('loop-status').textContent).toBe('The end must be after the start'));
  });

  it('a complete loop enables its buttons and the slider shows the position within', async () => {
    snap = bpmSnap();
    snap.transport = { ...idle, bpm: 120, position_ms: 6000 };
    render(PlayerView);
    await openTrack(snap);
    expect(btn('Loops', 'Play').disabled).toBe(false);
    expect(btn('Loops', 'Loop On').disabled).toBe(false);
    expect(input('bar1').value).toMatch(/^\d+\.\d+/);
    expect(input('bar2').value).toMatch(/^\d+\.\d+/);
    expect(screen.getByTestId('loop-slider').getAttribute('aria-valuenow')).toBe('50');
    expect(btn('Loops', 'Loop On').getAttribute('aria-pressed')).toBe('true');
  });

  it('the current loop being unreachable: loops exist but none is current (hand-edited player.json)', async () => {
    snap = { ...bpmSnap(), current_loop: null };
    render(PlayerView);
    await openTrack(snap);
    // The user must still be able to get to a loop with Next or Prev.
    const next = btn('Loops', 'Next');
    expect(next.disabled, 'Next is disabled although 2 loops exist and none is current').toBe(false);
  });
});

describe('AC24: boundary values typed in the boxes', () => {
  it('bar and time formats round trip', () => {
    expect(formatBar(0, 4)).toBe(parseBar(formatBar(0, 4), 4) === 0 ? formatBar(0, 4) : 'x');
    for (const t of [0, 1, 15, 16, 63, 64, 65, 500]) expect(parseBar(formatBar(t, 4), 4), String(t)).toBe(t);
    for (const bad of ['', 'x', '1.2.3.4', '-1.1.1', '1.0.0', '1.5.0', '99999999999999999999.1.0', '1.1.99999999']) {
      const v = parseBar(bad, 4);
      expect(v === null || Number.isFinite(v), bad).toBe(true);
    }
    for (const ms of [0, 1, 999, 1000, 59999, 60000, 3599999]) expect(parseLoopTime(formatLoopTime(ms)), String(ms)).toBe(ms);
    for (const bad of ['', 'abc', '1:2:3:4', '-1:00', '0:60.0', 'NaN', 'Infinity', '1e9']) {
      const v = parseLoopTime(bad);
      expect(v === null || (Number.isFinite(v) && v >= 0), bad).toBe(true);
    }
  });
});

describe('AC29, 1: leaving and Ctrl+L', () => {
  it('leaving the Player view while playing sends player_leave; nothing when idle', async () => {
    snap = base();
    render(App);
    ui.view = 'player';
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    selectTrack(practice.id);
    await waitFor(() => expect(pl.status).toBe('active'));
    await flush();
    ui.view = 'library';
    await flush();
    expect(of('player_leave')).toHaveLength(0);
    ui.view = 'player';
    await flush();
    pl.transport = { ...pl.transport, playing: true };
    ui.view = 'library';
    await flush();
    expect(of('player_leave')).toHaveLength(1);
  });

  it('Ctrl+L collapses and restores with the selection unchanged; Ctrl+Shift+L and Alt+Ctrl+L do not', async () => {
    snap = base();
    render(PlayerView);
    await openTrack(snap);
    const sel = lib.selectedId;
    await fireEvent.keyDown(window, { code: 'KeyL', ctrlKey: true, shiftKey: true });
    await tick();
    expect(screen.queryByTestId('tree-rail')).toBeNull();
    await fireEvent.keyDown(window, { code: 'KeyL', ctrlKey: true });
    await tick();
    expect(screen.getByTestId('tree-rail')).toBeTruthy();
    expect(lib.selectedId).toBe(sel);
    await fireEvent.keyDown(window, { code: 'KeyL', ctrlKey: true });
    await tick();
    expect(lib.selectedId).toBe(sel);
    expect(pl.status).toBe('active');
  });
});

describe('keys and focus', () => {
  it('Space and b/e/arrows act only outside text fields', async () => {
    snap = bpmSnap();
    render(PlayerView);
    await openTrack(snap);
    const f = input('BPM');
    f.focus();
    await fireEvent.keyDown(f, { code: 'Space', key: ' ' });
    await fireEvent.keyDown(f, { key: 'b' });
    await fireEvent.keyDown(f, { key: 'e' });
    await fireEvent.keyDown(f, { key: 'ArrowRight' });
    await flush();
    expect(of('player_play')).toHaveLength(0);
    expect(of('player_loop')).toHaveLength(0);
    (document.activeElement as HTMLElement)?.blur();
    await fireEvent.keyDown(document.body, { code: 'Space', key: ' ' });
    await fireEvent.keyDown(document.body, { key: 'b' });
    await fireEvent.keyDown(document.body, { key: 'e' });
    await fireEvent.keyDown(document.body, { key: 'ArrowRight' });
    await flush();
    expect(of('player_play')).toHaveLength(1);
    expect(of('player_loop').map((c) => (c.action as { kind: string }).kind)).toEqual(['mark', 'mark', 'select']);
  });

  it('on the focused waveform Space plays and the left arrow moves the change, not the loop; elsewhere the arrows select loops', async () => {
    snap = {
      ...bpmSnap(),
      tempo: { beats_per_bar: 4, multi: true, segments: [{ start_ms: 500, bpm: 120 }, { start_ms: 8000, bpm: 93, on_beat: true }] },
      transport: { ...idle, bpm: 93, tempo_index: 1 },
    };
    render(PlayerView);
    await openTrack(snap);
    selectSection(1);
    await flush();
    const wf = screen.getByTestId('player-waveform');
    wf.focus();
    await fireEvent.keyDown(wf, { code: 'Space', key: ' ' });
    await fireEvent.keyDown(wf, { key: 'ArrowLeft' });
    await flush();
    expect(of('player_play')).toHaveLength(1);
    expect(of('player_loop')).toHaveLength(0);
    expect(of('player_tempo').map((c) => (c.action as { kind: string }).kind)).toEqual(['move-change']);
    (document.activeElement as HTMLElement)?.blur();
    await fireEvent.keyDown(document.body, { key: 'ArrowLeft' });
    await flush();
    expect(of('player_loop').map((c) => (c.action as { kind: string }).kind)).toEqual(['select']);
  });

  it('Home and End jump in the target section from the page and the waveform, and type in the name box', async () => {
    snap = bpmSnap();
    render(PlayerView);
    await openTrack(snap);
    const edges = () => of('player_tempo').map((c) => (c.action as { kind: string; edge?: string }).edge);
    await fireEvent.keyDown(document.body, { key: 'Home' });
    await fireEvent.keyDown(document.body, { key: 'End' });
    await fireEvent.keyDown(document.body, { key: 'End', repeat: true });
    await flush();
    expect(edges()).toEqual(['start', 'end']);
    const wf = screen.getByTestId('player-waveform');
    wf.focus();
    await fireEvent.keyDown(wf, { key: 'Home' });
    await flush();
    expect(edges()).toEqual(['start', 'end', 'start']);
    const box = input('Section name');
    box.focus();
    await fireEvent.keyDown(box, { key: 'Home' });
    await fireEvent.keyDown(box, { key: 'End' });
    await fireEvent.keyDown(box, { code: 'Space', key: ' ' });
    await flush();
    expect(edges()).toHaveLength(3);
    expect(of('player_play')).toHaveLength(0);
  });

  it('with Ctrl or Alt held the plain keys do nothing', async () => {
    snap = bpmSnap();
    render(PlayerView);
    await openTrack(snap);
    await fireEvent.keyDown(document.body, { code: 'Space', key: ' ', ctrlKey: true });
    await fireEvent.keyDown(document.body, { key: 'b', altKey: true });
    await fireEvent.keyDown(document.body, { key: 'e', ctrlKey: true });
    await flush();
    expect(of('player_play')).toHaveLength(0);
    expect(of('player_loop')).toHaveLength(0);
  });
});

describe('AC28: settings file problems are shown to the user', () => {
  it('a note about an unreadable player.json is visible on the pane', async () => {
    snap = { ...base(), note: 'Player settings in player.json could not be read; they will be replaced when you change something' };
    render(PlayerView);
    await openTrack(snap);
    expect(document.body.textContent).toContain('could not be read');
  });

  it('a state-error event (player.json could not be written) is visible', async () => {
    snap = base();
    render(PlayerView);
    await openTrack(snap);
    channel!.onmessage({ kind: 'state-error', id: practice.id, message: 'Could not save the player settings: denied' } as PlayerEvent);
    await flush();
    await tick();
    expect(document.body.textContent).toContain('Could not save the player settings');
  });

  it('an audio-error event is visible and stops the playing state', async () => {
    snap = base();
    render(PlayerView);
    await openTrack(snap);
    channel!.onmessage({ kind: 'audio-error', id: practice.id, message: 'device gone' } as PlayerEvent);
    await flush();
    await tick();
    expect(document.body.textContent).toContain('device gone');
    expect(btn('Playback', 'Play')).toBeTruthy();
  });
});

describe('QA phase 2: section name box, Home / End and the section toggles (gui-player-tempo-sections)', () => {
  const kinds = () => of('player_tempo').map((c) => c.action as { kind: string; name?: string; edge?: string });

  it('qa_p2: Home / End do nothing without sections, and with a focused button or a modifier-free field', async () => {
    snap = base(); // no tempo
    render(PlayerView);
    await openTrack(snap);
    await fireEvent.keyDown(document.body, { key: 'Home' });
    await fireEvent.keyDown(document.body, { key: 'End' });
    await flush();
    expect(of('player_tempo')).toHaveLength(0);
    expect(btn('Sections', 'Jump to the start of the section').disabled).toBe(true);
    expect(input('Section name').disabled).toBe(true);
    expect(btn('Sections', 'Metronome in this section').disabled).toBe(true);
    expect(btn('Sections', 'Count-in into this section').disabled).toBe(true);
    // the master buttons keep working-state rules of phase 1 (not usable without a tempo map is their old behaviour)
  });

  it('qa_p2: Home / End on a focused button are left to the button', async () => {
    snap = bpmSnap();
    render(PlayerView);
    await openTrack(snap);
    const b = btn('Sections', 'Jump to the last bar of the section');
    b.focus();
    await fireEvent.keyDown(b, { key: 'End' });
    await fireEvent.keyDown(b, { key: 'Home' });
    await flush();
    expect(kinds()).toHaveLength(0);
  });

  it('qa_p2: a name typed with control characters or beyond 40 characters is sent cleaned; an emptied name clears', async () => {
    snap = bpmSnap();
    snap.tempo.segments[0] = { ...snap.tempo.segments[0], name: 'Old' };
    render(PlayerView);
    await openTrack(snap);
    const box = input('Section name');
    expect(box.value).toBe('Old');
    box.focus();
    await fireEvent.input(box, { target: { value: ' \tA\u0007' + 'é'.repeat(60) + ' ' } });
    await fireEvent.keyDown(box, { key: 'Enter' });
    await flush();
    expect(kinds()).toEqual([{ kind: 'section-name', index: 0, at: 500, name: 'A' + 'é'.repeat(39), } as never]);
    box.focus();
    await fireEvent.input(box, { target: { value: '   ' } });
    await fireEvent.keyDown(box, { key: 'Enter' });
    await flush();
    expect(kinds().map((k) => k.name)).toEqual(['A' + 'é'.repeat(39), '']);
  });

  it('qa_p2: while the name box has the focus its section stays even if the playhead moves on, and Escape sends nothing', async () => {
    snap = bpmSnap();
    snap.tempo.segments.push({ start_ms: 6000, bpm: 90 });
    snap.tempo.segments[1] = { ...snap.tempo.segments[1], name: 'Two' };
    render(PlayerView);
    await openTrack(snap);
    const box = input('Section name');
    box.focus();
    channel!.onmessage({ kind: 'transport', id: practice.id, ...idle, bpm: 90, tempo_index: 1 });
    await flush();
    expect(screen.getByTestId('section-status').textContent).toBe('Section 1 of 2 · 120 BPM');
    expect(box.value).toBe('');
    await fireEvent.input(box, { target: { value: 'zzz' } });
    await fireEvent.keyDown(box, { key: 'Escape' });
    await flush();
    expect(kinds()).toHaveLength(0);
    expect(box.value).toBe('Two'); // the box follows the playhead again
  });
});
