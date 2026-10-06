// QA acceptance tests for specs/gui-frontend-foundation.md, written from the spec text.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import App from './App.svelte';
import { ui } from '$lib/app-state.svelte';
import { resetLibrary } from '$lib/library-state.svelte';

const NAV_ORDER = ['Library', 'Import', 'Track', 'Playlists', 'Player', 'Settings'];
// Spec requirement 4: which future feature each view names.
const FEATURES: Record<string, string[]> = {
  // Library has no placeholder any more: gui-tracks-repository replaced it with the real view.
  // Import has no placeholder any more: gui-stem-extraction replaced it with the real view.
  Track: ['gui-backing-track-assembly', 'gui-tablatures', 'gui-manipulate-backing-track'],
  Player: ['gui-play-backing-track'],
};
let logs: string[] = [];

beforeEach(() => {
  logs = [];
  resetLibrary();
  ui.view = 'library';
  ui.theme = 'dark';
  ui.version = '';
  ui.navCollapsed = false;
  document.documentElement.classList.add('dark');
  mockIPC((cmd, args) => {
    if (cmd === 'app_version') return '26.10.1234';
    if (cmd === 'get_settings') return { theme: 'dark' };
    if (cmd === 'set_theme') return { theme: (args as { theme: string }).theme };
    if (cmd === 'get_repository') return { root: '/tmp/x', is_default: true, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/tmp/x', tracks: [], problems: [] };
    if (cmd === 'frontend_log') logs.push((args as { message: string }).message);
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

const nav = () => screen.getByRole('navigation');
const entry = (name: string) => nav().querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!;
const main = () => document.querySelector('main')!;

describe('acceptance: navigation (req 4, AC2, AC3)', () => {
  it('starts on dark theme with the six entries in the owner-decided order', () => {
    render(App);
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    const labels = [...nav().querySelectorAll('button[aria-label]')]
      .map((b) => b.getAttribute('aria-label'))
      .filter((l) => NAV_ORDER.includes(l!));
    expect(labels).toEqual(NAV_ORDER);
  });

  it('every view shows its placeholder naming the future feature (mouse)', async () => {
    render(App);
    for (const name of NAV_ORDER) {
      await fireEvent.click(entry(name));
      expect(main().querySelector('h1')!.textContent).toBe(name);
      const text = main().textContent!;
      if (name === 'Playlists') expect(text).toContain('Planned; there is no feature spec yet.');
      else if (name === 'Settings') {
        for (const t of ['MIDI interface', 'Audio output', 'Edge-AI server', 'Appearance']) expect(text).toContain(t);
        expect(text).toContain('gui-play-backing-track');
        expect(text).toContain('gui-stem-extracting');
      } else if (name === 'Import') expect(text).toContain('Stem Extraction');
      else if (name === 'Library') await waitFor(() => expect(main().textContent).toContain('No tracks in the repository yet.'));
      else for (const f of FEATURES[name]) expect(text).toContain(f);
    }
  });

  it('every view is reachable by Alt+digit including numpad; wrong modifiers do nothing', async () => {
    render(App);
    for (const [i, name] of NAV_ORDER.entries()) {
      ui.view = i === 0 ? 'settings' : 'library';
      await tick();
      await fireEvent.keyDown(window, { code: `Numpad${i + 1}`, altKey: true });
      await tick();
      expect(main().querySelector('h1')!.textContent).toBe(name);
    }
    ui.view = 'library';
    await tick();
    await fireEvent.keyDown(window, { code: 'Digit3', altKey: true, shiftKey: true });
    await fireEvent.keyDown(window, { code: 'Digit3', ctrlKey: true });
    await fireEvent.keyDown(window, { code: 'Digit3' });
    await fireEvent.keyDown(window, { code: 'Digit7', altKey: true });
    await tick();
    expect(main().querySelector('h1')!.textContent).toBe('Library');
  });

  it('Track view has tabs for stems, assembly, BPM/sections, tablature, MIDI cues', async () => {
    render(App);
    await fireEvent.click(entry('Track'));
    const tabs = screen.getAllByRole('tab').map((t) => t.textContent!.trim());
    expect(tabs).toEqual(['Stems', 'Assembly', 'BPM & sections', 'Tablature', 'MIDI cues']);
  });
});

describe('acceptance: keyboard (req 6)', () => {
  it('all navigation controls are native buttons, tabbable and not removed from the tab order', () => {
    render(App);
    for (const n of NAV_ORDER) {
      const b = entry(n);
      expect(b.tagName).toBe('BUTTON');
      expect(b.tabIndex).toBeGreaterThanOrEqual(0);
    }
    expect(screen.getByRole('button', { name: 'About calliope-gui' }).tabIndex).toBeGreaterThanOrEqual(0);
  });

  it('theme radios are keyboard operable controls', async () => {
    render(App);
    await fireEvent.click(entry('Settings'));
    expect(screen.getAllByRole('radio').length).toBe(2);
  });
});

describe('acceptance: version (req 5, AC4)', () => {
  it('footer and About show exactly the version returned by the Rust command', async () => {
    render(App);
    const btn = await screen.findByRole('button', { name: 'About calliope-gui' });
    await waitFor(() => expect(btn.textContent!.trim()).toBe('v26.10.1234'));
    await fireEvent.click(btn);
    expect((await screen.findByRole('dialog')).textContent).toContain('26.10.1234');
    expect(logs).toContain('ready view=library theme=dark version=26.10.1234');
  });

  it('footer degrades to v? when the command fails', async () => {
    clearMocks();
    mockIPC((cmd) => {
      if (cmd === 'app_version') throw new Error('boom');
      return undefined;
    });
    render(App);
    const btn = await screen.findByRole('button', { name: 'About calliope-gui' });
    await waitFor(() => expect(btn.textContent!.trim()).toBe('v?'));
  });
});

describe('acceptance: theme (AC5)', () => {
  it('switching to light then back to dark toggles the whole document', async () => {
    render(App);
    await fireEvent.click(entry('Settings'));
    await fireEvent.click(screen.getByRole('radio', { name: 'Light' }));
    await tick();
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    await fireEvent.click(screen.getByRole('radio', { name: 'Dark' }));
    await tick();
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('theme stays applied when changing views', async () => {
    render(App);
    await fireEvent.click(entry('Settings'));
    await fireEvent.click(screen.getByRole('radio', { name: 'Light' }));
    await fireEvent.click(entry('Library'));
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });
});
