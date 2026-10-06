import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  cancelImport, checkEdgeAi, checkTools, discardImport, getImportJob, importFile, prepareUrlImport,
  setEdgeAiUrl, setKeepOriginal, startStemExtraction, startUrlImport, watchImport,
  type ImportEvent, type JobSnapshot, type TrackEdits,
} from './ipc';

afterEach(() => clearMocks());

const edits: TrackEdits = {
  band: 'B', album: 'A', title: 'T', composers: ['C'], year: 2020, source_url: null, copyright: null,
};
const snapshot: JobSnapshot = {
  job: 'j1', source: { kind: 'url', label: 'https://x.example/a' }, phase: 'downloading',
  downloaded: 0, total: null, sent: 0, stems_done: 0, stems_total: 6, progress: null,
  duration_s: null, metadata: null, error: null, track: null,
};

describe('import ipc wrappers', () => {
  it('send the command names and argument names', async () => {
    const calls: [string, Record<string, unknown>][] = [];
    mockIPC((cmd, args) => { calls.push([cmd, args as Record<string, unknown>]); return null; });
    await setEdgeAiUrl('http://archserver:8765');
    await setEdgeAiUrl(null);
    await setKeepOriginal(true);
    await checkEdgeAi();
    await checkTools();
    await prepareUrlImport('https://x.example/a');
    await startUrlImport('https://x.example/a', true, () => {});
    await importFile('audio', () => {});
    await startStemExtraction('j1', edits);
    await cancelImport('j1');
    await discardImport('j1');
    await getImportJob();
    await watchImport(() => {});
    const plain = (i: number) => {
      const { events: _e, ...rest } = calls[i][1];
      return [calls[i][0], rest];
    };
    expect(calls.map((c) => c[0])).toEqual([
      'set_edge_ai_url', 'set_edge_ai_url', 'set_keep_original', 'check_edge_ai', 'check_tools',
      'prepare_url_import', 'start_url_import', 'import_file', 'start_stem_extraction',
      'cancel_import', 'discard_import', 'get_import_job', 'watch_import',
    ]);
    expect(calls[0][1]).toEqual({ url: 'http://archserver:8765' });
    expect(calls[1][1]).toEqual({ url: null });
    expect(calls[2][1]).toEqual({ keep: true });
    expect(calls[5][1]).toEqual({ url: 'https://x.example/a' });
    expect(plain(6)).toEqual(['start_url_import', { url: 'https://x.example/a', resume: true }]);
    expect(plain(7)).toEqual(['import_file', { kind: 'audio' }]);
    expect(calls[8][1]).toEqual({ job: 'j1', edits });
    expect(calls[9][1]).toEqual({ job: 'j1' });
    expect(calls[10][1]).toEqual({ job: 'j1' });
    expect(plain(12)).toEqual(['watch_import', {}]);
    for (const i of [6, 7, 12]) expect(JSON.stringify(calls[i][1].events)).toMatch(/^"__CHANNEL__:\d+"$/);
  });

  it('returns the snapshots, null for a cancelled dialog and the typed results', async () => {
    mockIPC((cmd) => {
      if (cmd === 'start_url_import' || cmd === 'cancel_import') return snapshot;
      if (cmd === 'prepare_url_import') return { status: 'partial', message: 'm', partial_bytes: 5 };
      return null;
    });
    expect((await startUrlImport('https://x.example/a', false, () => {})).job).toBe('j1');
    expect((await prepareUrlImport('https://x.example/a')).status).toBe('partial');
    expect(await importFile('video', () => {})).toBeNull();
    expect(await getImportJob()).toBeNull();
    expect((await cancelImport('j1')).source.kind).toBe('url');
  });

  it('delivers channel messages to the onEvent callback', async () => {
    const got: ImportEvent[] = [];
    mockIPC((cmd, args) => {
      if (cmd === 'watch_import') {
        const id = (args as { events: { id: number } }).events.id;
        const internals = (window as unknown as {
          __TAURI_INTERNALS__: { runCallback: (id: number, data: unknown) => void };
        }).__TAURI_INTERNALS__;
        internals.runCallback(id, { index: 0, message: { phase: 'working', progress: 0.5 } });
        internals.runCallback(id, { index: 1, message: { phase: 'queued' } });
      }
      return null;
    });
    await watchImport((e) => got.push(e));
    expect(got).toEqual([{ phase: 'working', progress: 0.5 }, { phase: 'queued' }]);
  });
});
