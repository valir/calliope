import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import LibraryView from './LibraryView.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { LibraryProblem, RepoStatus, TrackRecord } from '$lib/ipc';

let logs: string[] = [];
let tracks: TrackRecord[] = FIXTURE_TRACKS;
let problems: LibraryProblem[] = [];
let status: RepoStatus = 'ok';
let failList = false;

beforeEach(() => {
  logs = [];
  tracks = FIXTURE_TRACKS;
  problems = [];
  status = 'ok';
  failList = false;
  resetLibrary();
  mockIPC((cmd, args) => {
    if (cmd === 'get_repository') return { root: '/tmp/repo', is_default: true, status };
    if (cmd === 'list_tracks') {
      if (failList) throw new Error('disk on fire');
      return { root: '/tmp/repo', tracks, problems };
    }
    if (cmd === 'frontend_log') logs.push((args as { message: string }).message);
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

const items = () => screen.queryAllByRole('treeitem');
const labels = () => items().map((i) => i.textContent!.trim());
const search = () => screen.getByRole('searchbox', { name: 'Search tracks' }) as HTMLInputElement;

async function ready(): Promise<void> {
  render(LibraryView);
  await waitFor(() => expect(items().length).toBeGreaterThan(0));
}
const click = (name: string) => fireEvent.click(screen.getByRole('button', { name }));
const expandAll = async () => {
  await click('Expand all');
  await tick();
};

describe('Library view: structure', () => {
  it('has the heading, a Track section and the toolbar', async () => {
    await ready();
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Library');
    expect(screen.getByRole('region', { name: 'Track' })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Collapse all/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Expand all/ })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Clear search' }).hasAttribute('disabled')).toBe(true);
  });

  it('AC1: every band is collapsed at first', async () => {
    await ready();
    expect(items().length).toBe(4);
    expect(items().every((i) => i.getAttribute('aria-expanded') === 'false')).toBe(true);
    expect(labels()).toEqual(['Amber Fields', 'the Night Owls', 'Zephyr Lane', '(no band)']);
    expect(logs).toContain('library root=/tmp/repo status=ok tracks=6 problems=0');
  });

  it('AC2: labels and order at all three levels after Expand all', async () => {
    await ready();
    await expandAll();
    expect(labels()).toEqual([
      'Amber Fields',
      'Copper Sky',
      'Copper Sky',
      'Northern Roads',
      'after midnight',
      'Slow Burn',
      'the Night Owls',
      'Lanterns',
      'Lanterns',
      'Zephyr Lane',
      '(no album)',
      'Open Water',
      '(no band)',
      '(no album)',
      'Café Practice Groove',
    ]);
    expect(items().map((i) => i.getAttribute('aria-level'))).toEqual(
      ['1', '2', '3', '2', '3', '3', '1', '2', '3', '1', '2', '3', '1', '2', '3'],
    );
    await click('Collapse all');
    expect(items().length).toBe(4);
  });
});

describe('Library view: search', () => {
  it('AC5: filters on each input event and Clear restores the collapsed state', async () => {
    await ready();
    const box = search();
    let typed = '';
    const seen: string[][] = [];
    for (const ch of 'nght') {
      typed += ch;
      await fireEvent.input(box, { target: { value: typed } });
      seen.push(labels());
    }
    expect(seen[0]).toContain('the Night Owls');
    expect(seen[3]).toEqual(['the Night Owls', 'Lanterns', 'Lanterns']);
    expect(items().every((i) => i.getAttribute('aria-level') === '3' || i.getAttribute('aria-expanded') === 'true')).toBe(true);
    await click('Clear search');
    expect(search().value).toBe('');
    expect(items().length).toBe(4);
    expect(items().every((i) => i.getAttribute('aria-expanded') === 'false')).toBe(true);
  });

  it('shows a message when nothing matches', async () => {
    await ready();
    await fireEvent.input(search(), { target: { value: 'mbr' } });
    expect(items().length).toBe(0);
    expect(screen.getByText('No tracks match "mbr"')).toBeTruthy();
  });

  it('matches diacritic-insensitively and across tokens', async () => {
    await ready();
    await fireEvent.input(search(), { target: { value: 'cafe' } });
    expect(labels()).toEqual(['(no band)', '(no album)', 'Café Practice Groove']);
    await fireEvent.input(search(), { target: { value: 'owls lan' } });
    expect(labels()).toEqual(['the Night Owls', 'Lanterns', 'Lanterns']);
  });

  it('Ctrl+F focuses the search box and Down moves into the tree', async () => {
    await ready();
    await fireEvent.keyDown(window, { code: 'KeyF', ctrlKey: true });
    expect(document.activeElement).toBe(search());
    await fireEvent.keyDown(search(), { key: 'ArrowDown' });
    await tick();
    expect(document.activeElement).toBe(items()[0]);
  });
});

describe('Library view: tree keyboard', () => {
  it('roving tabindex, arrows, Enter selects and Left goes to the parent', async () => {
    await ready();
    expect(items().filter((i) => i.tabIndex === 0).length).toBe(1);
    const first = items()[0];
    first.focus();
    await fireEvent.keyDown(first, { key: 'ArrowRight' });
    expect(items()[0].getAttribute('aria-expanded')).toBe('true');
    expect(labels().slice(0, 3)).toEqual(['Amber Fields', 'Copper Sky', 'Northern Roads']);
    await fireEvent.keyDown(items()[0], { key: 'ArrowRight' });
    await tick();
    expect(document.activeElement).toBe(items()[1]);
    await fireEvent.keyDown(items()[1], { key: 'Enter' });
    expect(items()[1].getAttribute('aria-expanded')).toBe('true');
    await fireEvent.keyDown(items()[1], { key: 'ArrowDown' });
    await tick();
    const track = items()[2];
    expect(document.activeElement).toBe(track);
    expect(track.textContent!.trim()).toBe('Copper Sky');
    await fireEvent.keyDown(track, { key: ' ' });
    expect(items()[2].getAttribute('aria-selected')).toBe('true');
    expect(logs).toContain('select id=0199b0a0-0000-7000-8000-000000000003');
    await fireEvent.keyDown(items()[2], { key: 'ArrowLeft' });
    await tick();
    expect(document.activeElement).toBe(items()[1]);
    await fireEvent.keyDown(items()[1], { key: 'End' });
    await tick();
    expect(document.activeElement).toBe(items()[items().length - 1]);
    await fireEvent.keyDown(document.activeElement!, { key: 'Home' });
    await tick();
    expect(document.activeElement).toBe(items()[0]);
    await fireEvent.keyDown(items()[0], { key: 'ArrowLeft' });
    expect(items()[0].getAttribute('aria-expanded')).toBe('false');
  });

  it('click toggles groups and selects tracks', async () => {
    await ready();
    await fireEvent.click(items()[0]);
    expect(items()[0].getAttribute('aria-expanded')).toBe('true');
    await expandAll();
    const t = items().find((i) => i.textContent!.trim() === 'Slow Burn')!;
    await fireEvent.click(t);
    expect(t.getAttribute('aria-selected')).toBe('true');
  });
});

describe('Library view: states', () => {
  it('shows the empty state with the root path', async () => {
    tracks = [];
    render(LibraryView);
    await screen.findByText(/No tracks in the repository yet\./);
    expect(screen.getByTestId('library-root').textContent).toBe('/tmp/repo');
    expect(items().length).toBe(0);
  });

  it('the problems line expands into a list', async () => {
    problems = [
      { dir: 'bad-1', message: 'invalid JSON' },
      { dir: 'bad-2', message: 'schema 2' },
    ];
    await ready();
    const btn = screen.getByRole('button', { name: '2 track folders could not be read' });
    expect(screen.queryByText('bad-1: invalid JSON')).toBeNull();
    await fireEvent.click(btn);
    expect(screen.getByText('bad-1: invalid JSON')).toBeTruthy();
    expect(screen.getByText('bad-2: schema 2')).toBeTruthy();
    expect(logs).toContain('library root=/tmp/repo status=ok tracks=6 problems=2');
  });

  it('points to Settings when the repository is missing or newer', async () => {
    status = 'missing';
    render(LibraryView);
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('Settings');
    expect(alert.textContent).toContain('/tmp/repo');
  });

  it('reports a load failure inline and in the log', async () => {
    failList = true;
    render(LibraryView);
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('disk on fire');
    expect(logs.some((l) => l.startsWith('error list_tracks:'))).toBe(true);
  });
});
