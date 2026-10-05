import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import LibraryView from './LibraryView.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { Picked, SaveTrackRequest, TrackRecord } from '$lib/ipc';

let tracks: TrackRecord[] = [];
let calls: { cmd: string; args: unknown }[] = [];
let saves: SaveTrackRequest[] = [];
let pick: Picked | null = null;
let saveError: string | null = null;
let exportPath: string | null = null;
const NEW_MODIFIED = '2026-10-05T09:00:00Z';

beforeEach(() => {
  tracks = structuredClone(FIXTURE_TRACKS);
  calls = [];
  saves = [];
  pick = null;
  saveError = null;
  exportPath = '/tmp/out/export.zip';
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
        if (saveError) throw new Error(saveError);
        const t = tracks.find((x) => x.id === req.id)!;
        const updated: TrackRecord = {
          ...t,
          ...req.edits,
          tablatures: req.tablatures.map((e) => (e.kind === 'add' ? `added-${e.token}.gp5` : e.name)),
          modified: NEW_MODIFIED,
          revision: 'r-new',
        };
        tracks = tracks.map((x) => (x.id === t.id ? updated : x));
        return { track: updated, warnings: [] };
      }
      case 'delete_track':
        tracks = tracks.filter((t) => t.id !== (args as { id: string }).id);
        return undefined;
      case 'export_track':
      case 'export_tablature':
        return exportPath;
    }
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

const barBtn = (name: string) =>
  within(screen.getByRole('toolbar', { name: 'Track actions' })).getByRole('button', { name }) as HTMLButtonElement;
const tabBtn = (name: string) =>
  within(screen.getByRole('region', { name: 'Tablature Files' })).getByRole('button', { name }) as HTMLButtonElement;
const BAR = ['Edit', 'Save', 'Cancel', 'Export', 'Delete'];
const btn = (name: string) => (BAR.includes(name) ? barBtn(name) : tabBtn(name));
const anyBtn = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const field = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
const tabs = () => screen.queryAllByRole('option');
const clickBtn = async (name: string) => {
  await fireEvent.click(btn(name));
  await tick();
};

async function open(title: string | null): Promise<void> {
  render(LibraryView);
  await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
  if (title === null) return;
  await fireEvent.click(anyBtn('Expand all'));
  await tick();
  await fireEvent.click(screen.getByRole('treeitem', { name: title }));
  await tick();
}
const edit = async () => {
  await clickBtn('Edit');
  await tick();
};
const calledCmds = () => calls.map((c) => c.cmd);

describe('Track pane: states', () => {
  it('AC6: empty library shows empty disabled fields and disabled buttons', async () => {
    tracks = [];
    render(LibraryView);
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('No tracks'));
    for (const l of ['Band', 'Album', 'Title', 'Composers', 'Year', 'Source link', 'Copyright', 'Track ID', 'Modified']) {
      expect(field(l).disabled).toBe(true);
      expect(field(l).value).toBe('');
    }
    for (const b of ['Edit', 'Save', 'Cancel', 'Export', 'Delete', 'Add', 'Update', 'Remove']) {
      expect(btn(b).disabled).toBe(true);
    }
    expect(tabBtn('Export').disabled).toBe(true);
  });

  it('AC7: no selection shows the same inactive pane', async () => {
    await open(null);
    expect(field('Title').disabled).toBe(true);
    expect(field('Title').value).toBe('');
    for (const b of ['Edit', 'Save', 'Cancel', 'Export', 'Delete']) expect(btn(b).disabled).toBe(true);
    expect(btn('Add').disabled).toBe(true);
  });

  it('AC8: selection shows the metadata; Edit/Export/Delete on, Save off; bar after the fields', async () => {
    await open('Slow Burn');
    expect(field('Band').value).toBe('Amber Fields');
    expect(field('Title').value).toBe('Slow Burn');
    expect(field('Composers').value).toBe('Ann Example\nBo Sample');
    expect(field('Year').value).toBe('2019');
    expect(field('Track ID').value).toBe(FIXTURE_TRACKS[0].id);
    expect(field('Audio file').value).toBe('backing.mp3');
    expect(field('Modified').value).toBe('2026-10-01T12:00:00Z');
    expect(field('Title').readOnly).toBe(true);
    expect(btn('Edit').disabled).toBe(false);
    expect(btn('Export').disabled).toBe(false);
    expect(btn('Delete').disabled).toBe(false);
    expect(btn('Save').disabled).toBe(true);
    expect(btn('Cancel').disabled).toBe(true);
    const bar = screen.getByRole('toolbar', { name: 'Track actions' });
    expect(field('Copyright').compareDocumentPosition(bar) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(btn('Add').disabled).toBe(true);
  });

  it('AC9: Edit makes fields editable except the read-only ones; buttons switch', async () => {
    await open('Slow Burn');
    await edit();
    expect(field('Title').readOnly).toBe(false);
    expect(field('Composers').readOnly).toBe(false);
    expect(field('Track ID').readOnly).toBe(true);
    expect(field('Audio file').readOnly).toBe(true);
    expect(field('Modified').readOnly).toBe(true);
    expect(btn('Edit').disabled).toBe(true);
    expect(btn('Export').disabled).toBe(true);
    expect(btn('Delete').disabled).toBe(true);
    expect(btn('Save').disabled).toBe(false);
    expect(btn('Cancel').disabled).toBe(false);
    expect(btn('Add').disabled).toBe(false);
    expect((screen.getByRole('searchbox') as HTMLInputElement).disabled).toBe(true);
  });

  it('Ctrl+E edits, Escape cancels an unchanged edit without asking', async () => {
    await open('Slow Burn');
    await fireEvent.keyDown(window, { key: 'e', code: 'KeyE', ctrlKey: true });
    await tick();
    expect(btn('Save').disabled).toBe(false);
    await fireEvent.keyDown(window, { key: 'Escape', code: 'Escape' });
    await tick();
    expect(btn('Edit').disabled).toBe(false);
    expect(screen.queryByRole('alertdialog')).toBeNull();
  });
});

describe('Track pane: saving and discarding', () => {
  it('AC10: save sends the edits, shows the returned Modified, returns to view mode', async () => {
    await open('Slow Burn');
    await edit();
    await fireEvent.input(field('Title'), { target: { value: '  Slow Burn II ' } });
    await fireEvent.input(field('Year'), { target: { value: '' } });
    await fireEvent.input(field('Composers'), { target: { value: 'A\n\nB\n' } });
    await clickBtn('Save');
    await waitFor(() => expect(btn('Edit').disabled).toBe(false));
    expect(saves.length).toBe(1);
    expect(saves[0].id).toBe(FIXTURE_TRACKS[0].id);
    expect(saves[0].revision).toBe('r1');
    expect(saves[0].edits.title).toBe('Slow Burn II');
    expect(saves[0].edits.year).toBeNull();
    expect(saves[0].edits.composers).toEqual(['A', 'B']);
    expect(field('Modified').value).toBe(NEW_MODIFIED);
    expect(field('Title').readOnly).toBe(true);
    expect(screen.getByRole('status').textContent).toContain('Saved');
  });

  it('Ctrl+S saves', async () => {
    await open('Slow Burn');
    await edit();
    await fireEvent.keyDown(window, { key: 's', code: 'KeyS', ctrlKey: true });
    await waitFor(() => expect(saves.length).toBe(1));
  });

  it('a validation error stays in edit mode without calling save_track', async () => {
    await open('Slow Burn');
    await edit();
    await fireEvent.input(field('Title'), { target: { value: '  ' } });
    await clickBtn('Save');
    expect(saves.length).toBe(0);
    expect(screen.getByText('Title is required')).toBeTruthy();
    expect(field('Title').getAttribute('aria-invalid')).toBe('true');
    expect(btn('Save').disabled).toBe(false);
  });

  it('a save error stays in edit mode and shows the message', async () => {
    saveError = 'disk full';
    await open('Slow Burn');
    await edit();
    await fireEvent.input(field('Band'), { target: { value: 'X' } });
    await clickBtn('Save');
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('disk full'));
    expect(btn('Save').disabled).toBe(false);
    expect(field('Band').value).toBe('X');
  });

  it('a conflict reloads and leaves edit mode with a clear message', async () => {
    saveError = 'conflict: track changed on disk';
    await open('Slow Burn');
    await edit();
    await clickBtn('Save');
    await waitFor(() => expect(btn('Edit').disabled).toBe(false));
    expect(screen.getByRole('alert').textContent).toContain('changed on disk');
    expect(calledCmds().filter((c) => c === 'list_tracks').length).toBe(2);
  });

  it('Cancel with changes asks "Discard your changes?"; No keeps editing, Yes discards', async () => {
    await open('Slow Burn');
    await edit();
    await fireEvent.input(field('Title'), { target: { value: 'Changed' } });
    await clickBtn('Cancel');
    const dlg = await screen.findByRole('alertdialog');
    expect(dlg.textContent).toContain('Discard your changes?');
    await fireEvent.click(within(dlg).getByRole('button', { name: 'No' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(field('Title').value).toBe('Changed');
    expect(btn('Save').disabled).toBe(false);

    await fireEvent.keyDown(window, { key: 'Escape', code: 'Escape' });
    const dlg2 = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dlg2).getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(btn('Edit').disabled).toBe(false));
    expect(field('Title').value).toBe('Slow Burn');
    expect(saves.length).toBe(0);
  });
});

describe('Track pane: delete and export', () => {
  it('Delete confirms, then calls delete_track and clears the selection', async () => {
    await open('Slow Burn');
    await clickBtn('Delete');
    const dlg = await screen.findByRole('alertdialog');
    expect(dlg.textContent).toContain(
      'Delete track "Slow Burn"? It will be moved to the repository\'s trash folder.',
    );
    expect(calledCmds()).not.toContain('delete_track');
    await fireEvent.click(within(dlg).getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(calledCmds()).toContain('delete_track'));
    expect(calls.find((c) => c.cmd === 'delete_track')!.args).toEqual({
      id: FIXTURE_TRACKS[0].id,
      revision: 'r1',
    });
    await waitFor(() => expect(field('Title').value).toBe(''));
  });

  it('Delete answered No does nothing', async () => {
    await open('Slow Burn');
    await clickBtn('Delete');
    const dlg = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dlg).getByRole('button', { name: 'No' }));
    await tick();
    expect(calledCmds()).not.toContain('delete_track');
  });

  it('Export shows "Exported to <path>"; a cancelled dialog shows nothing', async () => {
    await open('Slow Burn');
    await clickBtn('Export');
    await waitFor(() => expect(screen.getByRole('status').textContent).toBe('Exported to /tmp/out/export.zip'));
    exportPath = null;
    await clickBtn('Export');
    await tick();
    expect(screen.queryByRole('status')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

describe('Tablature panel', () => {
  it('AC11: track 1 lists its 2 tablatures in edit mode', async () => {
    await open('Slow Burn');
    await edit();
    expect(tabs().map((o) => o.textContent!.trim())).toEqual(['slow-burn.gp5', 'slow-burn-solo.gp']);
    expect(screen.getByText('Tablature Files')).toBeTruthy();
  });

  it('AC12: track 2 has no tablatures; only Add is enabled', async () => {
    await open('after midnight');
    await edit();
    expect(tabs().length).toBe(0);
    expect(btn('Add').disabled).toBe(false);
    for (const b of ['Update', 'Export', 'Remove']) expect(tabBtn(b).disabled).toBe(true);
  });

  it('AC13: a selected row enables Update, Export and Remove', async () => {
    await open('Slow Burn');
    await edit();
    await fireEvent.click(tabs()[0]);
    await tick();
    expect(tabs()[0].getAttribute('aria-selected')).toBe('true');
    for (const b of ['Add', 'Update', 'Export', 'Remove']) expect(tabBtn(b).disabled).toBe(false);
  });

  it('AC14/AC15: Add calls pick_tablature("add") and selects the new row', async () => {
    pick = { token: 'p1', name: 'riff.gp5' };
    await open('after midnight');
    await edit();
    await clickBtn('Add');
    await waitFor(() => expect(tabs().length).toBe(1));
    expect(calls.find((c) => c.cmd === 'pick_tablature')!.args).toEqual({ purpose: 'add' });
    expect(tabs()[0].textContent).toContain('riff.gp5');
    expect(tabs()[0].textContent).toContain('new');
    expect(tabs()[0].getAttribute('aria-selected')).toBe('true');
    expect(btn('Remove').disabled).toBe(false);
  });

  it('a cancelled pick changes nothing', async () => {
    pick = null;
    await open('Slow Burn');
    await edit();
    await clickBtn('Add');
    await tick();
    expect(tabs().length).toBe(2);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('a name clash shows an error and adds nothing', async () => {
    pick = { token: 'p2', name: 'SLOW-BURN.GP5' };
    await open('Slow Burn');
    await edit();
    await clickBtn('Add');
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('already listed'));
    expect(tabs().length).toBe(2);
  });

  it('AC16: Save after Add sends {kind:"add", token}', async () => {
    pick = { token: 'p1', name: 'riff.gp5' };
    await open('after midnight');
    await edit();
    await clickBtn('Add');
    await waitFor(() => expect(tabs().length).toBe(1));
    await clickBtn('Save');
    await waitFor(() => expect(saves.length).toBe(1));
    expect(saves[0].tablatures).toContainEqual({ kind: 'add', token: 'p1' });
  });

  it('Update stages a replace of the selected row', async () => {
    pick = { token: 'p3', name: 'better.gp5' };
    await open('Slow Burn');
    await edit();
    await fireEvent.click(tabs()[0]);
    await clickBtn('Update');
    await waitFor(() => expect(tabs()[0].textContent).toContain('better.gp5'));
    expect(calls.find((c) => c.cmd === 'pick_tablature')!.args).toEqual({ purpose: 'update' });
    expect(tabs()[0].textContent).toContain('updated');
    await clickBtn('Save');
    await waitFor(() => expect(saves.length).toBe(1));
    expect(saves[0].tablatures[0]).toEqual({ kind: 'replace', name: 'slow-burn.gp5', token: 'p3' });
  });

  async function removeFirst(answer: 'Yes' | 'No'): Promise<HTMLElement> {
    await open('Slow Burn');
    await edit();
    await fireEvent.click(tabs()[0]);
    await clickBtn('Remove');
    const dlg = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dlg).getByRole('button', { name: answer }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    return dlg;
  }

  it('AC17: Remove asks Delete tablature "<name>"?', async () => {
    await open('Slow Burn');
    await edit();
    await fireEvent.click(tabs()[0]);
    await clickBtn('Remove');
    const dlg = await screen.findByRole('alertdialog');
    expect(dlg.textContent).toContain('Delete tablature "slow-burn.gp5"?');
  });

  it('AC18: Yes removes the row without any IPC call; No keeps it', async () => {
    await removeFirst('No');
    expect(tabs().length).toBe(2);
    cleanup();
    resetLibrary();
    calls = [];
    await removeFirst('Yes');
    expect(tabs().map((o) => o.textContent!.trim())).toEqual(['slow-burn-solo.gp']);
    expect(calledCmds()).not.toContain('save_track');
    expect(calledCmds()).not.toContain('delete_tablature');
    expect(btn('Remove').disabled).toBe(true);
  });

  it('AC19: Save after Remove omits the removed name', async () => {
    await removeFirst('Yes');
    await clickBtn('Save');
    await waitFor(() => expect(saves.length).toBe(1));
    expect(saves[0].tablatures).toEqual([{ kind: 'keep', name: 'slow-burn-solo.gp' }]);
  });

  it('tablature Export calls export_tablature for a saved row', async () => {
    await open('Slow Burn');
    await edit();
    await fireEvent.click(tabs()[1]);
    await fireEvent.click(tabBtn('Export'));
    await waitFor(() =>
      expect(calls.find((c) => c.cmd === 'export_tablature')!.args).toEqual({
        id: FIXTURE_TRACKS[0].id,
        name: 'slow-burn-solo.gp',
      }),
    );
  });

  it('Track 5 (5 tablatures) renders a scrollable list that fits 4 rows', async () => {
    await open('Open Water');
    await edit();
    expect(tabs().length).toBe(5);
    const list = screen.getByRole('listbox');
    expect(list.className).toContain('overflow-y-auto');
    expect(list.className).toContain('max-h-32'); // 8rem = 4 rows of h-8
    expect(tabs()[0].className).toContain('h-8');
  });
});
