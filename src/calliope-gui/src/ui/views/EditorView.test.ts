import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import EditorView from './EditorView.svelte';
import { lib, resetLibrary } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';

beforeEach(() => {
  resetLibrary();
  mockIPC((cmd) => {
    if (cmd === 'get_repository') return { root: '/tmp/repo', is_default: true, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/tmp/repo', tracks: structuredClone(FIXTURE_TRACKS), problems: [] };
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

describe('Editor view', () => {
  it('shows the shared tree, the Stems tab and no Assembly tab', async () => {
    render(EditorView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Editor');
    expect(screen.getByRole('tab', { name: 'Stems' })).toBeTruthy();
    expect(screen.queryByRole('tab', { name: 'Assembly' })).toBeNull();
    expect(screen.getByTestId('tree-toolbar')).toBeTruthy();
  });

  it('selecting a track in the Editor tree selects it in the Library state', async () => {
    render(EditorView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    await fireEvent.click(screen.getByRole('button', { name: 'Expand all' }));
    await tick();
    await fireEvent.click(screen.getByRole('treeitem', { name: /Glass Harbour$/ }));
    await tick();
    expect(lib.selectedId).toBe(FIXTURE_TRACKS[6].id);
  });
});
