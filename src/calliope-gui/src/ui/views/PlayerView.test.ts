import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import PlayerView from './PlayerView.svelte';
import SettingsView from './SettingsView.svelte';
import SourceSelect from '../components/player/SourceSelect.svelte';
import { lib, resetLibrary } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';

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
