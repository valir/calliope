import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import PlayerView from './PlayerView.svelte';
import SettingsView from './SettingsView.svelte';
import SourceSelect from '../components/player/SourceSelect.svelte';
import { lib, resetLibrary } from '$lib/library-state.svelte';
import { pl, resetPlayer } from '$lib/player-state.svelte';
import { FIXTURE_TRACKS, PLAYER_FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { PlayerLoop, PlayerSnapshot, PlayerTransport } from '$lib/ipc';

const [practice] = PLAYER_FIXTURE_TRACKS;

let commands: string[];
let logs: string[];
beforeEach(() => {
  resetLibrary();
  commands = [];
  logs = [];
  mockIPC((cmd, args) => {
    commands.push(cmd);
    if (cmd === 'frontend_log') logs.push(String((args as { message: string }).message));
    if (cmd === 'get_repository') return { root: '/tmp/repo', is_default: true, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/tmp/repo', tracks: structuredClone(FIXTURE_TRACKS), problems: [] };
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

describe('Player view', () => {
  it('shows the shared tree, rescans on mount, and selecting sets lib.selectedId', async () => {
    render(PlayerView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    expect(commands).toContain('list_tracks');
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Player');
    expect(screen.getByTestId('placeholder').textContent).toContain('gui-tablatures');
    await fireEvent.click(screen.getByRole('button', { name: 'Expand all' }));
    await tick();
    await fireEvent.click(screen.getByRole('treeitem', { name: /Glass Harbour$/ }));
    await tick();
    expect(lib.selectedId).toBe(FIXTURE_TRACKS[6].id);
  });

  it('Ctrl+L collapses the tree to a rail and the button or Ctrl+L restores it', async () => {
    render(PlayerView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    expect(document.getElementById('view-heading')).toBeTruthy();

    await fireEvent.keyDown(window, { code: 'KeyL', ctrlKey: true });
    await tick();
    expect(screen.queryAllByRole('treeitem')).toHaveLength(0);
    expect(screen.getByTestId('tree-rail')).toBeTruthy();
    expect(document.getElementById('view-heading')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Library' }).getAttribute('title')).toBe('Show the library (Ctrl+L)');
    expect(logs).toContain('player tree collapsed=true');

    await fireEvent.click(screen.getByRole('button', { name: 'Library' }));
    await tick();
    expect(screen.queryByTestId('tree-rail')).toBeNull();
    expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0);
    expect(document.getElementById('view-heading')).toBeTruthy();
    expect(logs).toContain('player tree collapsed=false');

    await fireEvent.keyDown(window, { code: 'KeyL', ctrlKey: true });
    await tick();
    expect(screen.getByTestId('tree-rail')).toBeTruthy();
    await fireEvent.keyDown(window, { code: 'KeyL', ctrlKey: true });
    await tick();
    expect(screen.queryByTestId('tree-rail')).toBeNull();
  });
});

describe('SourceSelect', () => {
  it('lists the given options and reports a change', async () => {
    const changes: string[] = [];
    render(SourceSelect, {
      sources: [
        { key: 'backing:backing', label: 'Backing' },
        { key: 'stem:drums', label: 'Drums' },
      ],
      value: 'backing:backing',
      onchange: (k: string) => changes.push(k),
    });
    const trigger = screen.getByRole('button', { name: 'Audio source' });
    expect(trigger.textContent).toContain('Backing');
    await fireEvent.keyDown(trigger, { key: 'ArrowDown' });
    await tick();
    const options = await screen.findAllByRole('option');
    expect(options.map((o) => o.textContent?.trim())).toEqual(['Backing', 'Drums']);
    await fireEvent.pointerUp(options[1]);
    await fireEvent.click(options[1]);
    await waitFor(() => expect(changes).toEqual(['stem:drums']));
  });
});

describe('Settings placeholders', () => {
  it('both name gui-audio-midi-settings', () => {
    render(SettingsView);
    const text = document.body.textContent!;
    expect(text.match(/This section will be filled by: gui-audio-midi-settings\./g)).toHaveLength(2);
    expect(text).not.toContain('gui-play-backing-track');
  });
});

describe('Player keyboard shortcuts', () => {
  const idle: PlayerTransport = {
    playing: false, counting_in: false, resume_pending: false, position_ms: 0, loop_playing: false, bpm: 0, tempo_index: null,
  };
  const complete: PlayerLoop = {
    id: 'l1', start: { tick: 64 }, end: { tick: 192 }, start_ms: 2000, end_ms: 10000, repeat: false, count_in: false,
  };
  const incomplete: PlayerLoop = { ...complete, id: 'l2', end: null, end_ms: null };
  let snap: PlayerSnapshot;
  let calls: [string, Record<string, unknown>][];
  const of = (cmd: string) => calls.filter((c) => c[0] === cmd).map((c) => c[1]);
  const flush = () => new Promise((r) => setTimeout(r, 0));

  async function open(loops: PlayerLoop[]) {
    snap = {
      id: practice.id, title: 'T', sources: [{ key: 'backing:backing', label: 'Backing' }], source: 'backing:backing',
      duration_ms: 20000, sample_rate: 44100, peaks: [], mode: 'bpm',
      tempo: { beats_per_bar: 4, segments: [{ start_ms: 0, bpm: 120 }], multi: false }, speed: 1,
      metronome: { on: false, count_in: false }, loops, current_loop: 0, loop_message: null,
      transport: { ...idle }, can_change_speed: false, note: null,
    };
    lib.selectedId = practice.id;
    render(PlayerView);
    await waitFor(() => expect(pl.status).toBe('active'));
    await flush();
  }

  beforeEach(() => {
    calls = [];
    resetPlayer();
    mockIPC((cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push([cmd, a]);
      if (cmd === 'open_player') return snap;
      if (cmd === 'player_loop') return snap;
      if (cmd.startsWith('player_')) return { ...idle };
      if (cmd === 'get_repository') return { root: '/r', is_default: false, status: 'ok' };
      if (cmd === 'list_tracks') return { root: '/r', tracks: [practice], problems: [] };
      return null;
    });
  });

  const key = async (init: KeyboardEventInit, target: Element | Window = window) => {
    await fireEvent.keyDown(target, init);
    await flush();
  };

  it('Space toggles play only outside fields, sliders, buttons, the select and the tree', async () => {
    await open([complete]);
    await key({ code: 'Space', key: ' ' });
    expect(of('player_play')).toHaveLength(1);
    await key({ code: 'Space', key: ' ' }, screen.getByRole('textbox', { name: 'BPM' }));
    await key({ code: 'Space', key: ' ' }, screen.getByTestId('loop-slider'));
    await key({ code: 'Space', key: ' ' }, screen.getByRole('button', { name: 'Prev' }));
    await key({ code: 'Space', key: ' ' }, screen.getByTestId('source-select'));
    await key({ code: 'Space', key: ' ' }, screen.getAllByRole('treeitem')[0]);
    expect(of('player_play')).toHaveLength(1);
    expect(of('player_pause')).toHaveLength(0);
  });

  it('auto-repeat of Space, b, e and l does nothing', async () => {
    await open([complete]);
    await key({ code: 'Space', key: ' ', repeat: true });
    await key({ key: 'b', code: 'KeyB', repeat: true });
    await key({ key: 'e', code: 'KeyE', repeat: true });
    await key({ key: 'l', code: 'KeyL', repeat: true });
    await key({ key: 'L', code: 'KeyL', shiftKey: true, repeat: true });
    expect(of('player_play')).toHaveLength(0);
    expect(of('player_loop')).toHaveLength(0);
  });

  it('b and e mark start and end', async () => {
    await open([complete]);
    await key({ key: 'b', code: 'KeyB' });
    await key({ key: 'e', code: 'KeyE' });
    expect(of('player_loop')).toMatchObject([
      { action: { kind: 'mark', edge: 'start' } },
      { action: { kind: 'mark', edge: 'end' } },
    ]);
    await key({ key: 'b', code: 'KeyB' }, screen.getByRole('textbox', { name: 'BPM' }));
    expect(of('player_loop')).toHaveLength(2);
  });

  it('arrows select the previous or next loop outside the tree', async () => {
    await open([complete]);
    await key({ key: 'ArrowLeft', code: 'ArrowLeft' });
    await key({ key: 'ArrowRight', code: 'ArrowRight' });
    expect(of('player_loop')).toMatchObject([
      { action: { kind: 'select', delta: -1 } },
      { action: { kind: 'select', delta: 1 } },
    ]);
    await key({ key: 'ArrowRight', code: 'ArrowRight' }, screen.getAllByRole('treeitem')[0]);
    await key({ key: 'ArrowLeft', code: 'ArrowLeft' }, screen.getByRole('tree'));
    expect(of('player_loop')).toHaveLength(2);
  });

  it('L toggles repeat on a complete loop and does nothing on an incomplete one', async () => {
    await open([complete]);
    await key({ key: 'L', code: 'KeyL', shiftKey: true });
    await key({ key: 'l', code: 'KeyL' });
    expect(of('player_loop')).toMatchObject([{ action: { kind: 'flags', repeat: true, count_in: false } }, { action: { kind: 'flags', repeat: true } }]);
    cleanup();
    calls = [];
    resetPlayer();
    await open([incomplete]);
    await key({ key: 'L', code: 'KeyL', shiftKey: true });
    expect(of('player_loop')).toHaveLength(0);
  });

  it('[ and ] do nothing in increment A', async () => {
    await open([complete]);
    await key({ key: '[', code: 'BracketLeft' });
    await key({ key: ']', code: 'BracketRight' });
    expect(calls.filter((c) => c[0] === 'player_tempo' || c[0] === 'player_loop')).toHaveLength(0);
  });

  it('Ctrl+L works with the focus on a field in the view', async () => {
    await open([complete]);
    await key({ code: 'KeyL', key: 'l', ctrlKey: true }, screen.getByRole('textbox', { name: 'BPM' }));
    expect(screen.getByTestId('tree-rail')).toBeTruthy();
    expect(of('player_loop')).toHaveLength(0);
  });
});
