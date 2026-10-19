import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  applyEvent, browse, extractDirectory, footerText, imp, jobRunning, resetImport, showInLibrary, step,
} from './import-state.svelte';
import { lib, resetLibrary } from './library-state.svelte';
import { ui } from './app-state.svelte';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { BatchItem, ImportEvent, JobSnapshot } from './ipc';

let calls: [string, Record<string, unknown>][] = [];
let dirSnapshot: JobSnapshot | null = null;
const track = { ...FIXTURE_TRACKS[0], id: 'trk-1', title: 'Song A' };
const savedItem = (index: number, name: string, dropped: { name: string; audible_ms: number }[] = []): BatchItem =>
  ({ index, name, result: 'saved', track: { ...track, id: `trk-${index + 1}` }, dropped });
const failedItem: BatchItem = { index: 1, name: 'b.mp3', result: 'failed', stage: 'prepare', message: 'bad file' };
const logs = () => calls.filter((c) => c[0] === 'frontend_log').map((c) => c[1].message as string);
const batchFile = (index: number, name: string): ImportEvent => ({ phase: 'batch-file', index, total: 3, name });

beforeEach(() => {
  calls = [];
  dirSnapshot = null;
  resetImport();
  resetLibrary();
  ui.view = 'import';
  mockIPC((cmd, args) => {
    calls.push([cmd, (args ?? {}) as Record<string, unknown>]);
    if (cmd === 'pick_import_file') return { path: '/m/album', name: 'album' };
    if (cmd === 'import_directory') return dirSnapshot;
    if (cmd === 'list_tracks') return { root: '/r', tracks: [track], problems: [] };
    return undefined;
  });
});
afterEach(() => clearMocks());

describe('directory import state', () => {
  it('batch-file on a stub job creates the batch; the steps go extract then done on finished', () => {
    applyEvent(batchFile(1, 'b.mp3'));
    expect(imp.job?.batch).toEqual({ dir: '', files: ['', 'b.mp3', ''], current: 1, items: [] });
    expect(step()).toBe('extract');
    expect(jobRunning()).toBe(true);
    applyEvent({ phase: 'preparing' });
    expect(step()).toBe('extract'); // preparing and uploading are not the source page in a batch
    applyEvent({ phase: 'finished', batch: { dir: '/m', files: ['a'], current: null, items: [savedItem(0, 'a')] } });
    expect(step()).toBe('done');
    expect(jobRunning()).toBe(false);
    expect(footerText()).toBe('Import finished');
    expect(logs()).toContain('import batch file=2/3 name=b.mp3');
    expect(logs()).toContain('import batch finished saved=1 failed=0');
  });

  it('items are logged and the footer shows the file number', () => {
    applyEvent(batchFile(0, 'a.flac'));
    applyEvent({ phase: 'batch-item', item: savedItem(0, 'a.flac', [{ name: 'piano', audible_ms: 1 }]) });
    applyEvent({ phase: 'batch-item', item: savedItem(2, 'c') });
    applyEvent({ phase: 'batch-item', item: failedItem });
    applyEvent(batchFile(1, 'b.mp3'));
    applyEvent({ phase: 'working', progress: null });
    expect(imp.job?.batch?.items).toHaveLength(3);
    expect(logs()).toEqual(expect.arrayContaining([
      'import batch item=1 result=saved id=trk-1 dropped=piano',
      'import batch item=3 result=saved id=trk-3 dropped=none',
      'import batch item=2 result=failed stage=prepare message=bad file',
    ]));
    expect(footerText()).toBe('File 2 of 3: Working...');
  });

  it('cancelled returns to the source page with the notice built from the items', () => {
    applyEvent(batchFile(0, 'a'));
    applyEvent({ phase: 'batch-item', item: savedItem(0, 'a') });
    applyEvent({ phase: 'cancelled', back_to: 'source' });
    expect(imp.job).toBeNull();
    expect(step()).toBe('source');
    expect(imp.notice).toBe('Directory import cancelled: 1 of 3 files saved.');
    expect(logs()).toContain('import batch cancelled saved=1 of=3');
  });

  it('showInLibrary selects the first saved item, or only switches the view', () => {
    applyEvent(batchFile(0, 'a'));
    applyEvent({ phase: 'batch-item', item: failedItem });
    applyEvent({ phase: 'batch-item', item: savedItem(2, 'c') });
    applyEvent({ phase: 'batch-item', item: savedItem(0, 'a') });
    showInLibrary();
    expect(ui.view).toBe('library');
    expect(lib.selectedId).toBe('trk-3');
    expect(imp.job).toBeNull();

    ui.view = 'import';
    lib.selectedId = null;
    applyEvent(batchFile(0, 'a'));
    applyEvent({ phase: 'batch-item', item: failedItem });
    showInLibrary();
    expect(ui.view).toBe('library');
    expect(lib.selectedId).toBeNull();
  });

  it('adopt keeps the batch the events already built', async () => {
    dirSnapshot = {
      job: 'j9', source: { kind: 'directory', label: 'album' }, phase: 'preparing', downloaded: 0, total: null,
      sent: 0, stems_done: 0, stems_total: 0, progress: null, duration_s: null, metadata: null, error: null,
      track: null, dropped: [], batch: { dir: '/m/album', files: ['x', 'y', 'z'], current: 0, items: [] },
    };
    imp.path = '/m/album';
    // the event arrives while the command is still in flight, as the real channel does
    const p = extractDirectory();
    applyEvent(batchFile(0, 'a.flac'));
    await p;
    expect(imp.job?.job).toBe('j9');
    expect(imp.job?.source.kind).toBe('directory');
    expect(imp.job?.batch?.files).toEqual(['a.flac', '', '']);
    expect(imp.job?.batch?.dir).toBe('/m/album');
  });

  it('browse sends the box text as typed', async () => {
    imp.path = '/m/al';
    expect(await browse('directory')).toBe(true);
    expect(calls.find((c) => c[0] === 'pick_import_file')?.[1]).toEqual({ kind: 'directory', typed: '/m/al' });
    expect(imp.path).toBe('/m/album');
  });

  it('an empty directory box shows the folder message and makes no IPC call', async () => {
    await extractDirectory();
    expect(imp.error).toBe('Enter the path of a folder or choose one with Browse');
    expect(calls.map((c) => c[0])).not.toContain('import_directory');
  });
});
