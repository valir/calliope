// QA acceptance tests for gui-tracks-repository (frontend side). Independent of the
// implementer's component tests: own data, spec wording, edge cases.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import LibraryView from './views/LibraryView.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import type { Picked, SaveTrackRequest, TrackRecord } from '$lib/ipc';

const rec = (n: number, p: Partial<TrackRecord>): TrackRecord => ({
  id: `id-${n}`,
  band: 'B',
  album: 'A',
  title: `T${n}`,
  composers: [],
  year: null,
  source_url: null,
  copyright: null,
  audio: 'backing.mp3',
  tablatures: [],
  imported: '2026-10-01T12:00:00Z',
  modified: '2026-10-01T12:00:00Z',
  revision: `rev${n}`,
  missing: [],
  ...p,
});

let tracks: TrackRecord[] = [];
let calls: { cmd: string; args: unknown }[] = [];
let saves: SaveTrackRequest[] = [];
let pick: Picked | null = null;

const SORT_SET = (): TrackRecord[] => [
  rec(1, { band: 'zebra', album: 'Zed', title: 'z-song' }),
  rec(2, { band: 'Alpha', album: 'beta', title: 'Charlie' }),
  rec(3, { band: 'alpha', album: 'Beta', title: 'bravo' }),
  rec(4, { band: 'Alpha', album: 'Alpha', title: 'delta' }),
  rec(5, { band: 'Alpha', album: 'beta', title: 'Alice' }),
  rec(6, { band: 'Mike', album: 'x', title: 'Track 10' }),
  rec(7, { band: 'Mike', album: 'x', title: 'Track 9' }),
  rec(8, { band: '', album: '', title: 'Orphan' }),
];

beforeEach(() => {
  tracks = SORT_SET();
  calls = [];
  saves = [];
  pick = null;
  resetLibrary();
  mockIPC((cmd, args) => {
    if (cmd !== 'frontend_log') calls.push({ cmd, args });
    switch (cmd) {
      case 'get_repository':
        return { root: '/tmp/repo', is_default: true, status: 'ok' };
      case 'list_tracks':
        return { root: '/tmp/repo', tracks, problems: [] };
      case 'pick_tablature':
        return pick;
      case 'save_track': {
        const req = (args as { request: SaveTrackRequest }).request;
        saves.push(req);
        const t = tracks.find((x) => x.id === req.id)!;
        const u = { ...t, ...req.edits, modified: '2026-10-05T09:00:00Z', revision: 'new' };
        return { track: u, warnings: [] };
      }
    }
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
const btn = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const field = (l: string) => screen.getByLabelText(l) as HTMLInputElement;
const bar = (n: string) =>
  within(screen.getByRole('toolbar', { name: 'Track actions' })).getByRole('button', { name: n }) as HTMLButtonElement;
const tabBtn = (n: string) =>
  within(screen.getByRole('region', { name: 'Tablature Files' })).getByRole('button', { name: n }) as HTMLButtonElement;
const options = () => screen.queryAllByRole('option');

async function ready() {
  render(LibraryView);
  await waitFor(() => expect(items().length).toBeGreaterThan(0));
}
async function openTrack(title: string) {
  await ready();
  await fireEvent.click(btn('Expand all'));
  await tick();
  await fireEvent.click(screen.getByRole('treeitem', { name: title }));
  await tick();
}
async function edit() {
  await fireEvent.click(bar('Edit'));
  await tick();
}

describe('AC1/AC2: tree', () => {
  it('starts collapsed: only band rows, all aria-expanded=false', async () => {
    await ready();
    expect(items().length).toBe(4);
    for (const i of items()) expect(i.getAttribute('aria-expanded')).toBe('false');
  });

  it('bands sort case-insensitively, case variants group, (no band) last', async () => {
    await ready();
    expect(labels()).toEqual(['Alpha', 'Mike', 'zebra', '(no band)']);
  });

  it('three levels sorted case-insensitively and numerically after Expand all', async () => {
    await ready();
    await fireEvent.click(btn('Expand all'));
    await tick();
    const l = labels();
    expect(l).toEqual([
      'Alpha',
      'Alpha', // album
      'delta',
      'Beta', // grouped case variants: first spelling in exact order
      'Alice',
      'bravo',
      'Charlie',
      'Mike',
      'x',
      'Track 9',
      'Track 10',
      'zebra',
      'Zed',
      'z-song',
      '(no band)',
      '(no album)',
      'Orphan',
    ]);
    // levels
    const levels = items().map((i) => i.getAttribute('aria-level'));
    expect(levels.slice(0, 5)).toEqual(['1', '2', '3', '2', '3']);
  });
});

describe('AC3/AC4: expand all / collapse all', () => {
  it('expand all expands every group; collapse all collapses every group', async () => {
    await ready();
    await fireEvent.click(btn('Expand all'));
    await tick();
    const groups = items().filter((i) => i.hasAttribute('aria-expanded'));
    expect(groups.length).toBe(4 + 5);
    for (const g of groups) expect(g.getAttribute('aria-expanded')).toBe('true');
    await fireEvent.click(btn('Collapse all'));
    await tick();
    expect(items().length).toBe(4);
    for (const g of items()) expect(g.getAttribute('aria-expanded')).toBe('false');
  });
});

describe('AC5 / req 12: search', () => {
  it('filters on each keystroke, case-insensitive and fuzzy; clear restores', async () => {
    tracks = [
      rec(1, { band: 'the Night Owls', album: 'Lanterns', title: 'Lanterns' }),
      rec(2, { band: 'Amber Fields', album: 'Northern Roads', title: 'Slow Burn' }),
      rec(3, { band: '', album: '', title: 'Café Practice Groove' }),
    ];
    await ready();
    const typed: number[] = [];
    let v = '';
    for (const ch of 'NGHT') {
      v += ch;
      await fireEvent.input(search(), { target: { value: v } });
      await tick();
      typed.push(items().filter((i) => i.getAttribute('aria-level') === '3').length);
    }
    expect(typed[typed.length - 1]).toBe(1);
    expect(labels()).toContain('Lanterns');
    expect(labels()).not.toContain('Slow Burn');
    // matching branches are shown expanded while searching
    await fireEvent.input(search(), { target: { value: 'CAFE' } });
    await tick();
    expect(labels()).toContain('Café Practice Groove');
    await fireEvent.input(search(), { target: { value: 'zzzz' } });
    await tick();
    expect(items().length).toBe(0);
    expect(document.body.textContent).toContain('No tracks match "zzzz"');
    await fireEvent.click(btn('Clear search'));
    await tick();
    expect(search().value).toBe('');
    expect(items().length).toBe(3);
    for (const i of items()) expect(i.getAttribute('aria-expanded')).toBe('false');
  });

  it('whitespace-only query shows everything; regex metacharacters are plain text', async () => {
    await ready();
    await fireEvent.input(search(), { target: { value: '   ' } });
    await tick();
    expect(items().length).toBe(4);
    for (const q of ['.*', '(', '[a-', '\\', '+', '?']) {
      await fireEvent.input(search(), { target: { value: q } });
      await tick();
      expect(document.body.textContent).toContain('No tracks match');
    }
  });

  it('Clear button is disabled when the box is empty', async () => {
    await ready();
    expect(btn('Clear search').disabled).toBe(true);
  });
});

describe('AC6-AC9: pane states', () => {
  it('empty library: fields visible, all disabled; buttons disabled', async () => {
    tracks = [];
    render(LibraryView);
    await waitFor(() => expect(document.body.textContent).toContain('No tracks in the repository yet'));
    for (const l of ['Band', 'Album', 'Title', 'Composers', 'Year', 'Source link', 'Copyright', 'Track ID']) {
      expect(field(l)).toBeTruthy();
      expect(field(l).disabled).toBe(true);
    }
    for (const b of ['Edit', 'Save', 'Export', 'Delete']) expect(bar(b).disabled).toBe(true);
  });

  it('selection: values shown, only Save disabled, bar after fields; Edit toggles states', async () => {
    tracks[0] = { ...tracks[0], composers: ['Ann', 'Bo'], year: 1999, tablatures: ['a.gp5'] };
    await openTrack('z-song');
    expect(field('Title').value).toBe('z-song');
    expect(field('Composers').value).toBe('Ann\nBo');
    expect(field('Year').value).toBe('1999');
    expect(bar('Edit').disabled).toBe(false);
    expect(bar('Export').disabled).toBe(false);
    expect(bar('Delete').disabled).toBe(false);
    expect(bar('Save').disabled).toBe(true);
    expect(tabBtn('Add').disabled).toBe(true);
    await edit();
    expect(field('Track ID').readOnly || field('Track ID').disabled).toBe(true);
    expect(field('Title').readOnly).toBe(false);
    expect(bar('Edit').disabled).toBe(true);
    expect(bar('Export').disabled).toBe(true);
    expect(bar('Delete').disabled).toBe(true);
    expect(bar('Save').disabled).toBe(false);
    expect(tabBtn('Add').disabled).toBe(false);
    expect(options().map((o) => o.textContent!.trim())).toEqual(['a.gp5']);
  });

  it('metadata containing HTML is rendered as text', async () => {
    tracks = [rec(1, { band: '<img src=x onerror=alert(1)>', title: '<b>bold</b>' })];
    await ready();
    expect(document.querySelector('img[src="x"]')).toBeNull();
    await fireEvent.click(btn('Expand all'));
    await tick();
    expect(document.querySelector('b')).toBeNull();
  });

  it('source link is never rendered as an anchor', async () => {
    tracks = [rec(1, { source_url: 'https://example.org/x' })];
    await openTrack('T1');
    expect(document.querySelector('a[href]')).toBeNull();
  });

  it('tree and search are locked in edit mode', async () => {
    await openTrack('z-song');
    await edit();
    expect(search().disabled).toBe(true);
    expect(btn('Clear search').disabled).toBe(true);
  });
});

describe('AC10: save', () => {
  it('sends only edited-field values plus id/revision; never a path; returns to view mode', async () => {
    await openTrack('z-song');
    await edit();
    await fireEvent.input(field('Album'), { target: { value: 'New Album' } });
    await fireEvent.click(bar('Save'));
    await waitFor(() => expect(bar('Edit').disabled).toBe(false));
    expect(saves.length).toBe(1);
    expect(saves[0].edits.album).toBe('New Album');
    expect(saves[0].revision).toBe('rev1');
    expect(JSON.stringify(saves[0])).not.toMatch(/\/(home|tmp|etc)/);
    expect(bar('Save').disabled).toBe(true);
    expect(field('Modified').value).toBe('2026-10-05T09:00:00Z');
    expect(field('Album').readOnly).toBe(true);
  });

  it.each([
    ['Year', 'abc'],
    ['Year', '0'],
    ['Year', '10000'],
    ['Year', '1.5'],
    ['Year', '-3'],
    ['Title', ''],
    ['Title', 'x'.repeat(201)],
    ['Composers', Array.from({ length: 21 }, (_, i) => `c${i}`).join('\n')],
  ])('invalid %s=%j blocks the save and stays in edit mode', async (label, value) => {
    await openTrack('z-song');
    await edit();
    await fireEvent.input(field(label), { target: { value } });
    await fireEvent.click(bar('Save'));
    await tick();
    expect(saves.length).toBe(0);
    expect(bar('Save').disabled).toBe(false);
    expect(field('Title').readOnly).toBe(false);
  });
});

describe('AC11-AC19: tablatures', () => {
  const withTabs = () => {
    tracks[0] = { ...tracks[0], tablatures: ['one.gp5', 'two.gp'] };
  };

  it('AC12: no tablatures, only Add active', async () => {
    await openTrack('z-song');
    await edit();
    expect(tabBtn('Add').disabled).toBe(false);
    for (const b of ['Update', 'Export', 'Remove']) expect(tabBtn(b).disabled).toBe(true);
  });

  it('AC15: picked name (path-free) is added, selected, Remove active; AC16 request carries the token', async () => {
    pick = { token: 'p7', name: 'riff.gp5' };
    await openTrack('z-song');
    await edit();
    await fireEvent.click(tabBtn('Add'));
    await waitFor(() => expect(options().length).toBe(1));
    expect(options()[0].textContent).toContain('riff.gp5');
    expect(options()[0].getAttribute('aria-selected')).toBe('true');
    expect(tabBtn('Remove').disabled).toBe(false);
    await fireEvent.click(bar('Save'));
    await waitFor(() => expect(saves.length).toBe(1));
    expect(saves[0].tablatures).toEqual([{ kind: 'add', token: 'p7' }]);
  });

  it('a clash (case-insensitive) with a listed tablature, the audio file or track.json is refused', async () => {
    withTabs();
    await openTrack('z-song');
    await edit();
    for (const name of ['ONE.gp5', 'Backing.MP3', 'track.json']) {
      pick = { token: 'pX', name };
      await fireEvent.click(tabBtn('Add'));
      await tick();
      await waitFor(() => expect(screen.queryByRole('alert')).not.toBeNull());
      expect(options().length).toBe(2);
    }
    await fireEvent.click(bar('Save'));
    await waitFor(() => expect(saves.length).toBe(1));
    expect(saves[0].tablatures.every((t) => t.kind === 'keep')).toBe(true);
  });

  it('AC17/AC18/AC19: Remove asks, Yes drops the row without IPC, Save omits it', async () => {
    withTabs();
    await openTrack('z-song');
    await edit();
    await fireEvent.click(options()[0]);
    await tick();
    await fireEvent.click(tabBtn('Remove'));
    const dlg = await screen.findByRole('alertdialog');
    expect(dlg.textContent).toContain('Delete tablature "one.gp5"?');
    expect(within(dlg).getByRole('button', { name: 'Yes' })).toBeTruthy();
    expect(within(dlg).getByRole('button', { name: 'No' })).toBeTruthy();
    await fireEvent.click(within(dlg).getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(options().length).toBe(1));
    expect(calls.map((c) => c.cmd)).not.toContain('save_track');
    // selection cleared => Remove/Update disabled again
    expect(tabBtn('Remove').disabled).toBe(true);
    await fireEvent.click(bar('Save'));
    await waitFor(() => expect(saves.length).toBe(1));
    expect(saves[0].tablatures).toEqual([{ kind: 'keep', name: 'two.gp' }]);
  });

  it('Remove answered No keeps the row', async () => {
    withTabs();
    await openTrack('z-song');
    await edit();
    await fireEvent.click(options()[0]);
    await tick();
    await fireEvent.click(tabBtn('Remove'));
    const dlg = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dlg).getByRole('button', { name: 'No' }));
    await tick();
    expect(options().length).toBe(2);
  });

  it('adding then removing a staged tablature leaves no trace in the request', async () => {
    pick = { token: 'p1', name: 'tmp.gp5' };
    await openTrack('z-song');
    await edit();
    await fireEvent.click(tabBtn('Add'));
    await waitFor(() => expect(options().length).toBe(1));
    await fireEvent.click(tabBtn('Remove'));
    const dlg = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dlg).getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(options().length).toBe(0));
    await fireEvent.click(bar('Save'));
    await waitFor(() => expect(saves.length).toBe(1));
    expect(saves[0].tablatures).toEqual([]);
  });

  it('a missing-on-disk tablature is marked and cannot be exported', async () => {
    tracks[0] = { ...tracks[0], tablatures: ['gone.gp5'], missing: ['gone.gp5'] };
    await openTrack('z-song');
    await edit();
    expect(options()[0].textContent).toContain('missing');
    await fireEvent.click(options()[0]);
    await tick();
    expect(tabBtn('Export').disabled).toBe(true);
  });

  it('Cancel after staging changes asks first and discards on Yes (nothing saved)', async () => {
    withTabs();
    await openTrack('z-song');
    await edit();
    await fireEvent.click(options()[0]);
    await tick();
    await fireEvent.click(tabBtn('Remove'));
    let dlg = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dlg).getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(options().length).toBe(1));
    await fireEvent.click(bar('Cancel'));
    dlg = await screen.findByRole('alertdialog');
    expect(dlg.textContent).toContain('Discard your changes?');
    await fireEvent.click(within(dlg).getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(bar('Edit').disabled).toBe(false));
    expect(saves.length).toBe(0);
    await edit();
    expect(options().length).toBe(2);
  });
});

describe('Delete track', () => {
  it('confirmation focuses No; Escape does not delete', async () => {
    await openTrack('z-song');
    await fireEvent.click(bar('Delete'));
    const dlg = await screen.findByRole('alertdialog');
    expect(dlg.textContent).toContain('Delete track "z-song"?');
    expect(document.activeElement).toBe(within(dlg).getByRole('button', { name: 'No' }));
    await fireEvent.keyDown(dlg, { key: 'Escape', code: 'Escape' });
    await tick();
    expect(calls.map((c) => c.cmd)).not.toContain('delete_track');
  });
});
