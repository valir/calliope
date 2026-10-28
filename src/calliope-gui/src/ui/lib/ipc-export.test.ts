import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  cancelExport, chooseRc5Folder, exportInfo, getExport, prepareExport, startExport, watchExport, type ExportEvent,
} from './ipc';

afterEach(() => clearMocks());

describe('export ipc wrappers', () => {
  it('send the command names and argument names', async () => {
    const calls: [string, Record<string, unknown>][] = [];
    mockIPC((cmd, args) => { calls.push([cmd, args as Record<string, unknown>]); return null; });
    await exportInfo('t1');
    await prepareExport('t1', { kind: 'rc5', memory: '7', reuse: null });
    await prepareExport('t1', { kind: 'rc5', memory: '8', reuse: '/media/B' });
    await prepareExport('t1', { kind: 'mp3' });
    expect(await chooseRc5Folder()).toBeNull();
    await startExport('tok', true, () => {});
    await cancelExport('j1');
    await getExport();
    await watchExport(() => {});
    expect(calls.map((c) => c[0])).toEqual([
      'export_info', 'prepare_export', 'prepare_export', 'prepare_export', 'choose_rc5_folder', 'start_export', 'cancel_export', 'get_export', 'watch_export',
    ]);
    expect(calls[0][1]).toEqual({ id: 't1' });
    expect(calls[1][1]).toEqual({ id: 't1', target: { kind: 'rc5', memory: '7', reuse: null } });
    expect(calls[2][1]).toEqual({ id: 't1', target: { kind: 'rc5', memory: '8', reuse: '/media/B' } });
    expect(calls[3][1]).toEqual({ id: 't1', target: { kind: 'mp3' } });
    expect(Object.keys(calls[5][1]).sort()).toEqual(['events', 'overwrite', 'token']);
    expect(calls[5][1].token).toBe('tok');
    expect(calls[5][1].overwrite).toBe(true);
    expect(calls[6][1]).toEqual({ job: 'j1' });
    expect(Object.keys(calls[8][1])).toEqual(['events']);
    for (const i of [5, 8]) expect(JSON.stringify(calls[i][1].events)).toMatch(/^"__CHANNEL__:\d+"$/);
  });

  it('delivers channel messages to the onEvent callback', async () => {
    const got: ExportEvent[] = [];
    mockIPC((cmd, args) => {
      if (cmd === 'start_export') {
        const id = (args as { events: { id: number } }).events.id;
        const internals = (window as unknown as {
          __TAURI_INTERNALS__: { runCallback: (id: number, data: unknown) => void };
        }).__TAURI_INTERNALS__;
        internals.runCallback(id, { index: 0, message: { phase: 'writing', progress: 0.5 } });
        internals.runCallback(id, { index: 1, message: { phase: 'cancelled' } });
      }
      return null;
    });
    await startExport('tok', false, (e) => got.push(e));
    expect(got).toEqual([{ phase: 'writing', progress: 0.5 }, { phase: 'cancelled' }]);
  });
});
