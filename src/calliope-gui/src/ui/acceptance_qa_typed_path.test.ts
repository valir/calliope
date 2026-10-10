// QA acceptance (frontend) for specs/gui-existing-track-import.md, through the rendered UI with
// the Tauri IPC mocked at command level.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import ImportView from './views/ImportView.svelte';
import { imp, resetImport } from '$lib/import-state.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { ui } from '$lib/app-state.svelte';
import type { ImportEvent, JobSnapshot, TrackEdits } from '$lib/ipc';

type Call = [string, Record<string, unknown>];
let calls: Call[] = [];
let channel: ((e: ImportEvent) => void) | null = null;
let pick: { path: string; name: string } | null = null;
let importErr = '';

const META: TrackEdits = {
  band: 'Amber Fields', album: 'Northern Roads', title: 'Slow Burn', composers: ['Ann Example'],
  year: 2019, source_url: null, copyright: null,
};
const snap = (label: string, over: Partial<JobSnapshot> = {}): JobSnapshot => ({
  job: 'job-1', source: { kind: 'file', label }, phase: 'preparing', downloaded: 0, total: null,
  sent: 0, stems_done: 0, stems_total: 0, progress: null, duration_s: null, metadata: null, error: null,
  track: null, dropped: [], ...over,
} as JobSnapshot);
const names = () => calls.map((c) => c[0]);
const of = (cmd: string) => calls.filter((c) => c[0] === cmd).map((c) => c[1]);
const emit = (e: ImportEvent) => act(() => channel!(e));
const click = (role: 'button' | 'radio', name: string) => fireEvent.click(screen.getByRole(role, { name }));
const alertText = () => screen.queryByRole('alert')?.textContent?.trim() ?? null;

beforeEach(() => {
  calls = []; channel = null; pick = null; importErr = '';
  resetImport(); resetLibrary(); ui.view = 'import';
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (a.events) channel = (a.events as { onmessage: (e: ImportEvent) => void }).onmessage;
    switch (cmd) {
      case 'pick_import_file': return pick;
      case 'import_path':
        if (importErr) throw importErr;
        return snap(String(a.path).split('/').pop()!);
      case 'prepare_url_import': return { status: 'ready', message: '', partial_bytes: 0 };
      case 'start_url_import': return snap('u', { phase: 'downloading', source: { kind: 'url', label: 'u' } } as never);
      case 'start_stem_extraction': return snap('x', { phase: 'uploading', metadata: a.edits as TrackEdits });
      default: return undefined;
    }
  });
});
afterEach(() => { cleanup(); clearMocks(); });

async function open(label: string): Promise<HTMLInputElement | null> {
  render(ImportView);
  await click('button', 'Stem Extraction');
  await click('radio', label);
  return screen.queryByRole('textbox', { name: 'file path' }) as HTMLInputElement | null;
}

describe('req 1 and 2: path box + Browse per kind', () => {
  it.each([['Local Audio File', 'audio', '/path/to/song.mp3'], ['Local Video File', 'video', '/path/to/video.mp4']])(
    '%s shows box + Browse + Extract and hides the URL box', async (label, _k, ph) => {
      const box = await open(label);
      expect(box).toBeTruthy();
      expect(box!.placeholder).toBe(ph);
      expect(screen.getByRole('button', { name: 'Browse' })).toBeTruthy();
      expect(screen.getByRole('button', { name: 'Extract' })).toBeTruthy();
      expect(screen.queryByRole('textbox', { name: 'enter url' })).toBeNull();
    });
  it('URL shows the URL box and no Browse / path box', async () => {
    const box = await open('URL');
    expect(box).toBeNull();
    expect(screen.getByRole('textbox', { name: 'enter url' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Browse' })).toBeNull();
  });
  it.each([['Local Audio File', 'audio'], ['Local Video File', 'video']])('Browse (%s) only fills the box', async (label, kind) => {
    const box = (await open(label))!;
    pick = { path: '/home/me/My Music/a b.mp3', name: 'a b.mp3' };
    await click('button', 'Browse');
    await waitFor(() => expect(box.value).toBe('/home/me/My Music/a b.mp3'));
    expect(of('pick_import_file')).toEqual([{ kind }]);
    expect(names()).not.toContain('import_path');
    expect(document.activeElement?.textContent?.trim()).toBe('Extract');
    expect(imp.job).toBeNull();
  });
  it('a cancelled Browse keeps the typed text', async () => {
    const box = (await open('Local Audio File'))!;
    await fireEvent.input(box, { target: { value: '/typed/x.mp3' } });
    await click('button', 'Browse');
    await waitFor(() => expect(names()).toContain('pick_import_file'));
    expect(box.value).toBe('/typed/x.mp3');
    expect(names()).not.toContain('import_path');
  });
});

describe('AC1 / AC2 through the UI', () => {
  it.each([['Local Audio File', 'audio', '/m/song.mp3'], ['Local Video File', 'video', '~/Videos/"my clip".mp4']])(
    '%s: typed path + Enter reaches the edit pane and then extraction with the same job', async (label, kind, path) => {
      const box = (await open(label))!;
      await fireEvent.input(box, { target: { value: path } });
      await fireEvent.keyDown(box, { key: 'Enter' });
      await waitFor(() => expect(of('import_path')).toHaveLength(1));
      expect(of('import_path')[0]).toMatchObject({ kind, path });
      await emit({ phase: 'ready', metadata: META, duration_s: 100 });
      await screen.findByRole('textbox', { name: 'Title' });
      await click('button', 'Extract');
      await waitFor(() => expect(names()).toContain('start_stem_extraction'));
      expect(of('start_stem_extraction')[0]).toMatchObject({ job: 'job-1' });
    });
  it('Browse then Enter on the focused Extract imports the picked path', async () => {
    const box = (await open('Local Audio File'))!;
    pick = { path: '/m/p.flac', name: 'p.flac' };
    await click('button', 'Browse');
    await waitFor(() => expect(box.value).toBe('/m/p.flac'));
    await click('button', 'Extract');
    await waitFor(() => expect(of('import_path')).toEqual([{ kind: 'audio', path: '/m/p.flac', events: expect.anything() }]));
  });
});

describe('rejections', () => {
  it('empty and whitespace-only box: exact message, no IPC', async () => {
    const box = (await open('Local Audio File'))!;
    await click('button', 'Extract');
    expect(alertText()).toBe('Enter the path of a file or choose one with Browse');
    await fireEvent.input(box, { target: { value: '   ' } });
    await fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(alertText()).toBe('Enter the path of a file or choose one with Browse'));
    expect(names()).not.toContain('import_path');
  });
  it.each(['No file found at /x/y.mp3', '/x is a folder, not a file', '"a.mp3" is not a video file', 'Cannot read /x/y.mp3 (permission denied)', '/dev/null is not a regular file', 'Enter the full path, starting with / or ~/', 'The path is not valid'])(
    'rejected path "%s" is shown, box kept, controls enabled, no job', async (m) => {
      importErr = m;
      const box = (await open('Local Video File'))!;
      await fireEvent.input(box, { target: { value: '/x/y.mp3' } });
      await click('button', 'Extract');
      await waitFor(() => expect(alertText()).toBe(m));
      expect(box.value).toBe('/x/y.mp3');
      expect(box.disabled).toBe(false);
      expect((screen.getByRole('button', { name: 'Browse' }) as HTMLButtonElement).disabled).toBe(false);
      expect(imp.job).toBeNull();
      // retry clears the old message
      importErr = '';
      await click('button', 'Extract');
      await waitFor(() => expect(imp.job).not.toBeNull());
      expect(alertText()).toBeNull();
    });
  it('a video without audio shows the job failure and allows another try', async () => {
    const box = (await open('Local Video File'))!;
    await fireEvent.input(box, { target: { value: '/m/silent.mp4' } });
    await click('button', 'Extract');
    await waitFor(() => expect(imp.job).not.toBeNull());
    await emit({ phase: 'failed', stage: 'prepare', message: 'Selected file silent.mp4 has no audio track', http_status: null });
    await waitFor(() => expect(alertText()).toBe('Selected file silent.mp4 has no audio track'));
  });
});

describe('state clearing and regressions', () => {
  it('switching audio -> video -> URL -> audio clears the path and error', async () => {
    const box = (await open('Local Audio File'))!;
    importErr = 'No file found at /q.mp3';
    await fireEvent.input(box, { target: { value: '/q.mp3' } });
    await click('button', 'Extract');
    await waitFor(() => expect(alertText()).not.toBeNull());
    await click('radio', 'Local Video File');
    const box2 = screen.getByRole('textbox', { name: 'file path' }) as HTMLInputElement;
    expect(box2.value).toBe('');
    expect(alertText()).toBeNull();
    await fireEvent.input(box2, { target: { value: '/v.mp4' } });
    await click('radio', 'URL');
    await click('radio', 'Local Audio File');
    expect((screen.getByRole('textbox', { name: 'file path' }) as HTMLInputElement).value).toBe('');
  });
  it('URL import still works and sends no path', async () => {
    await open('URL');
    const u = screen.getByRole('textbox', { name: 'enter url' });
    await fireEvent.input(u, { target: { value: 'https://video.example/watch?v=1' } });
    await click('button', 'Extract');
    await waitFor(() => expect(names()).toContain('start_url_import'));
    expect(names()).not.toContain('import_path');
    expect(names()).not.toContain('pick_import_file');
  });
  it('controls are disabled while a job exists', async () => {
    const box = (await open('Local Audio File'))!;
    await fireEvent.input(box, { target: { value: '/m/a.mp3' } });
    await click('button', 'Extract');
    await waitFor(() => expect(imp.job).not.toBeNull());
    expect((screen.getByRole('textbox', { name: 'file path' }) as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Browse' }) as HTMLButtonElement).disabled).toBe(true);
  });
  it('resetImport clears the path', () => {
    imp.path = '/leftover.mp3';
    resetImport();
    expect(imp.path).toBe('');
  });
});
