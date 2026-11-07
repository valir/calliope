import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import {
  cancelExport, chooseRc5Folder, chooseRc500Folder, exportInfo, getExport, prepareExport, setSlowCountIn, startExport, watchExport, type ExportEvent,
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
    await prepareExport('t1', { kind: 'rc500', memory: '7', track: 2, reuse: null });
    expect(await chooseRc5Folder()).toBeNull();
    expect(await chooseRc500Folder()).toBeNull();
    await startExport('tok', true, () => {});
    await cancelExport('j1');
    await getExport();
    await watchExport(() => {});
    await setSlowCountIn('t1', true);
    expect(calls.map((c) => c[0])).toEqual([
      'export_info', 'prepare_export', 'prepare_export', 'prepare_export', 'prepare_export', 'choose_rc5_folder',
      'choose_rc500_folder', 'start_export', 'cancel_export', 'get_export', 'watch_export', 'set_slow_count_in',
    ]);
    expect(calls[0][1]).toEqual({ id: 't1' });
    expect(calls[1][1]).toEqual({ id: 't1', target: { kind: 'rc5', memory: '7', reuse: null } });
    expect(calls[2][1]).toEqual({ id: 't1', target: { kind: 'rc5', memory: '8', reuse: '/media/B' } });
    expect(calls[3][1]).toEqual({ id: 't1', target: { kind: 'mp3' } });
    expect(calls[4][1]).toEqual({ id: 't1', target: { kind: 'rc500', memory: '7', track: 2, reuse: null } });
    expect(Object.keys(calls[7][1]).sort()).toEqual(['events', 'overwrite', 'token']);
    expect(calls[7][1].token).toBe('tok');
    expect(calls[7][1].overwrite).toBe(true);
    expect(calls[8][1]).toEqual({ job: 'j1' });
    expect(Object.keys(calls[10][1])).toEqual(['events']);
    expect(calls[11][1]).toEqual({ id: 't1', on: true });
    for (const i of [7, 10]) expect(JSON.stringify(calls[i][1].events)).toMatch(/^"__CHANNEL__:\d+"$/);
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
