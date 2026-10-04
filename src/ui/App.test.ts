import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import App from './App.svelte';
import { ui } from '$lib/app-state.svelte';
import { VIEWS } from '$lib/views';

let calls: { cmd: string; args: unknown }[] = [];

beforeEach(() => {
  calls = [];
  ui.view = 'library';
  ui.theme = 'dark';
  ui.version = '';
  ui.navCollapsed = false;
  document.documentElement.classList.add('dark');
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === 'app_version') return '26.10.0042';
    if (cmd === 'get_settings') return { theme: 'dark' };
    if (cmd === 'set_theme') return { theme: (args as { theme: string }).theme };
    return undefined;
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

const navButtons = () => within_nav().getAllByRole('button').slice(1);
function within_nav() {
  return within(screen.getByRole('navigation'));
}

describe('App shell', () => {
  it('shows six nav buttons in order', () => {
    render(App);
    expect(navButtons().map((b) => b.getAttribute('aria-label'))).toEqual(VIEWS.map((v) => v.label));
  });

  it('switches view on click and names the features', async () => {
    render(App);
    for (const v of VIEWS) {
      await fireEvent.click(screen.getByRole('button', { name: v.label }));
      expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(v.label);
      expect(ui.view).toBe(v.id);
    }
    await fireEvent.click(screen.getByRole('button', { name: 'Import' }));
    expect(screen.getByTestId('placeholder').textContent).toContain('gui-stem-extracting');
    await fireEvent.click(screen.getByRole('button', { name: 'Playlists' }));
    expect(screen.getByTestId('placeholder').textContent).toContain('no feature spec');
    await fireEvent.click(screen.getByRole('button', { name: 'Track' }));
    expect(screen.getAllByTestId('placeholder')[0].textContent).toContain('gui-backing-track-assembly');
  });

  it('Alt+digit switches the view and focuses the heading', async () => {
    render(App);
    for (const v of VIEWS) {
      await fireEvent.keyDown(window, { code: `Digit${v.digit}`, altKey: true });
      await tick();
      expect(ui.view).toBe(v.id);
      const h = screen.getByRole('heading', { level: 1 });
      expect(h.textContent).toBe(v.label);
      expect(document.activeElement).toBe(h);
    }
  });

  it('shows the version in the footer and opens/closes About', async () => {
    render(App);
    const btn = await screen.findByRole('button', { name: 'About calliope-gui' });
    await waitFor(() => expect(btn.textContent?.trim()).toBe('v26.10.0042'));
    await fireEvent.click(btn);
    const dialog = await screen.findByRole('dialog');
    expect(dialog.textContent).toContain('26.10.0042');
    await fireEvent.keyDown(dialog, { key: 'Escape' });
    await tick();
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('choosing Light applies the theme and persists it', async () => {
    render(App);
    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    await fireEvent.click(screen.getByRole('radio', { name: 'Light' }));
    await tick();
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    expect(ui.theme).toBe('light');
    expect(calls.find((c) => c.cmd === 'set_theme')?.args).toEqual({ theme: 'light' });
    expect(calls.some((c) => c.cmd === 'frontend_log' && (c.args as { message: string }).message === 'theme=light')).toBe(true);
  });

  it('Ctrl+B collapses the nav but keeps labels', async () => {
    render(App);
    const toggle = screen.getByRole('button', { name: 'Collapse navigation' });
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    await fireEvent.keyDown(window, { code: 'KeyB', ctrlKey: true });
    await tick();
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(navButtons().map((b) => b.getAttribute('aria-label'))).toEqual(VIEWS.map((v) => v.label));
    expect(navButtons()[0].textContent?.trim()).toBe('');
  });

  it('marks the active entry with aria-current', async () => {
    render(App);
    await fireEvent.click(screen.getByRole('button', { name: 'Player' }));
    const current = navButtons().filter((b) => b.getAttribute('aria-current') === 'page');
    expect(current.map((b) => b.getAttribute('aria-label'))).toEqual(['Player']);
  });

  it('logs ready once and view on change only', async () => {
    render(App);
    const msgs = () => calls.filter((c) => c.cmd === 'frontend_log').map((c) => (c.args as { message: string }).message);
    await waitFor(() => expect(msgs()).toEqual(['ready view=library theme=dark version=26.10.0042']));
    await fireEvent.click(screen.getByRole('button', { name: 'Track' }));
    await tick();
    expect(msgs()).toContain('view=track');
  });
});
