import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import RepositorySettings from './RepositorySettings.svelte';
import { lib, resetLibrary } from '$lib/library-state.svelte';
import type { PickedRoot, RepoInfo } from '$lib/ipc';

let calls: string[] = [];
let logs: string[] = [];
let current: RepoInfo;
let picked: PickedRoot | null;

beforeEach(() => {
  calls = [];
  logs = [];
  current = { root: '/data/calliope', is_default: true, status: 'ok' };
  picked = null;
  resetLibrary();
  mockIPC((cmd, args) => {
    calls.push(cmd);
    if (cmd === 'get_repository') return current;
    if (cmd === 'choose_repository_root') return picked;
    if (cmd === 'set_repository_root') return { root: picked!.root, is_default: false, status: picked!.status };
    if (cmd === 'reset_repository_root') return { root: '/data/calliope', is_default: true, status: 'empty' };
    if (cmd === 'frontend_log') logs.push((args as { message: string }).message);
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

const click = (name: string) => fireEvent.click(screen.getByRole('button', { name }));

describe('RepositorySettings', () => {
  it('shows the path and (default)', async () => {
    render(RepositorySettings);
    expect((await screen.findByTestId('repository-root')).textContent).toBe('/data/calliope');
    expect(screen.getByText('(default)')).toBeTruthy();
  });

  it('shows a status line for a missing folder', async () => {
    current = { root: '/gone', is_default: false, status: 'missing' };
    render(RepositorySettings);
    expect((await screen.findByTestId('repository-status')).textContent).toContain('does not exist');
    expect(screen.queryByText('(default)')).toBeNull();
  });

  it('cancelling the dialog does nothing', async () => {
    render(RepositorySettings);
    await screen.findByTestId('repository-root');
    await click('Choose folder…');
    await waitFor(() => expect(calls).toContain('choose_repository_root'));
    expect(calls).not.toContain('set_repository_root');
  });

  it('an ok folder is set, the library is cleared and the change logged', async () => {
    picked = { token: 't1', root: '/music/repo', status: 'ok' };
    lib.loaded = true;
    render(RepositorySettings);
    await screen.findByTestId('repository-root');
    await click('Choose folder…');
    await waitFor(() => expect(screen.getByTestId('repository-root').textContent).toBe('/music/repo'));
    expect(lib.loaded).toBe(false);
    expect(logs).toContain('repository root=/music/repo default=false');
    expect(screen.queryByText('(default)')).toBeNull();
  });

  it('an other folder asks first; No makes no set call', async () => {
    picked = { token: 't2', root: '/home/x', status: 'other' };
    render(RepositorySettings);
    await screen.findByTestId('repository-root');
    await click('Choose folder…');
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog.textContent).toContain('Use "/home/x" as the track repository?');
    await click('No');
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(calls).not.toContain('set_repository_root');
  });

  it('an other folder asks first; Yes sets it', async () => {
    picked = { token: 't2', root: '/home/x', status: 'other' };
    render(RepositorySettings);
    await screen.findByTestId('repository-root');
    await click('Choose folder…');
    await screen.findByRole('alertdialog');
    await click('Yes');
    await waitFor(() => expect(calls).toContain('set_repository_root'));
    await waitFor(() => expect(screen.getByTestId('repository-root').textContent).toBe('/home/x'));
  });

  it('a newer folder shows an error and is not set', async () => {
    picked = { token: 't3', root: '/new', status: 'newer' };
    render(RepositorySettings);
    await screen.findByTestId('repository-root');
    await click('Choose folder…');
    expect((await screen.findByRole('alert')).textContent).toContain('newer Calliope');
    expect(calls).not.toContain('set_repository_root');
  });

  it('Use default calls the reset', async () => {
    current = { root: '/music/repo', is_default: false, status: 'ok' };
    render(RepositorySettings);
    await screen.findByTestId('repository-root');
    await click('Use default');
    await waitFor(() => expect(calls).toContain('reset_repository_root'));
    await waitFor(() => expect(screen.getByText('(default)')).toBeTruthy());
    expect(logs).toContain('repository root=/data/calliope default=true');
  });

  it('blocks switching while a track is being edited', async () => {
    lib.mode = 'edit';
    render(RepositorySettings);
    await screen.findByTestId('repository-root');
    await click('Use default');
    expect((await screen.findByRole('alert')).textContent).toContain('track edit');
    expect(calls).not.toContain('reset_repository_root');
    expect(calls).not.toContain('choose_repository_root');
  });
});
