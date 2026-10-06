import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import ImportView from './ImportView.svelte';
import StatusFooter from '../components/StatusFooter.svelte';
import { imp, resetImport } from '$lib/import-state.svelte';
import { lib, resetLibrary } from '$lib/library-state.svelte';
import { ui } from '$lib/app-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { ImportEvent, JobSnapshot, TrackEdits, TrackRecord, UrlPrep } from '$lib/ipc';

type Call = [string, Record<string, unknown>];
let calls: Call[] = [];
let channel: ((e: ImportEvent) => void) | null = null;
let prep: UrlPrep = { status: 'ready', message: '', partial_bytes: 0 };
let fileResult: JobSnapshot | null = null;
let watchResult: JobSnapshot | null = null;
let startError = '';

const URL_OK = 'https://video.example/watch?v=1';
const META: TrackEdits = {
  band: 'Amber Fields', album: 'Northern Roads', title: 'Slow Burn', composers: ['Ann Example', 'Bo Sample'],
  year: 2019, source_url: URL_OK, copyright: null,
};
const snap = (over: Partial<JobSnapshot> = {}): JobSnapshot => ({
  job: 'job-1', source: { kind: 'url', label: URL_OK }, phase: 'downloading', downloaded: 0, total: null,
  sent: 0, stems_done: 0, stems_total: 0, progress: null, duration_s: null, metadata: null, error: null,
  track: null, ...over,
});
const stemTrack: TrackRecord = {
  ...FIXTURE_TRACKS[0], type: 'stem', audio: null,
  stems: ['vocals', 'drums', 'bass', 'guitar', 'piano', 'other'].map((n) => ({ name: n, file: `${n}.flac` })),
  stem_model: 'htdemucs_6s',
};

const names = () => calls.map((c) => c[0]);
const callsOf = (cmd: string) => calls.filter((c) => c[0] === cmd).map((c) => c[1]);
const emit = (e: ImportEvent) => act(() => channel!(e));

beforeEach(() => {
  calls = [];
  channel = null;
  prep = { status: 'ready', message: '', partial_bytes: 0 };
  fileResult = null;
  watchResult = null;
  startError = '';
  resetImport();
  resetLibrary();
  ui.view = 'import';
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (a.events) channel = (a.events as { onmessage: (e: ImportEvent) => void }).onmessage;
    switch (cmd) {
      case 'prepare_url_import': return prep;
      case 'start_url_import':
        if (startError) throw new Error(startError);
        return snap();
      case 'import_file': return fileResult;
      case 'watch_import': return watchResult;
      case 'start_stem_extraction': return snap({ phase: 'uploading', metadata: a.edits as TrackEdits });
      case 'get_settings': return { theme: 'dark', repository_root: null, edge_ai_url: null, keep_original: false };
      case 'get_repository': return { root: '/tmp/r', is_default: true, status: 'ok' };
      case 'list_tracks': return { root: '/tmp/r', tracks: [stemTrack], problems: [] };
      default: return undefined;
    }
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

const click = (role: 'button' | 'radio', name: string) => fireEvent.click(screen.getByRole(role, { name }));
const alertText = () => screen.queryByRole('alert')?.textContent?.trim() ?? null;

async function toSource(): Promise<void> {
  render(ImportView);
  await click('button', 'Stem Extraction');
}
async function toUrlBox(): Promise<HTMLInputElement> {
  await toSource();
  await click('radio', 'URL');
  return screen.getByRole('textbox', { name: 'enter url' }) as HTMLInputElement;
}
async function startDownload(): Promise<void> {
  const box = await toUrlBox();
  await fireEvent.input(box, { target: { value: URL_OK } });
  await click('button', 'Extract');
  await waitFor(() => expect(names()).toContain('start_url_import'));
}
async function toEditPane(): Promise<void> {
  await startDownload();
  await emit({ phase: 'preparing' });
  await emit({ phase: 'ready', metadata: META, duration_s: 213 });
  await screen.findByRole('textbox', { name: 'Title' });
}
const field = (label: string) => screen.getByLabelText(label) as HTMLInputElement | HTMLTextAreaElement;

describe('Import view: menu and source page', () => {
  it('shows the Stem Extraction button and no placeholder', () => {
    render(ImportView);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Import');
    expect(screen.getByRole('button', { name: 'Stem Extraction' })).toBeTruthy();
    expect(screen.queryByTestId('placeholder')).toBeNull();
    expect(screen.queryByText(/This view will be filled by/)).toBeNull();
  });

  it('click opens the source page with the three sources and no box yet', async () => {
    await toSource();
    for (const n of ['URL', 'Local Audio File', 'Local Video File']) expect(screen.getByRole('radio', { name: n })).toBeTruthy();
    expect(screen.queryByRole('textbox', { name: 'enter url' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Browse' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Stem Extraction' })).toBeNull();
  });

  it('Back returns to the menu', async () => {
    await toSource();
    await click('button', 'Back');
    expect(screen.getByRole('button', { name: 'Stem Extraction' })).toBeTruthy();
  });

  it('URL shows the box with the "enter url" placeholder and an Extract button', async () => {
    const box = await toUrlBox();
    expect(box.placeholder).toBe('enter url');
    expect(screen.getByRole('button', { name: 'Extract' })).toBeTruthy();
  });

  it('a malformed URL shows the accent text under the box and starts nothing', async () => {
    const box = await toUrlBox();
    await fireEvent.input(box, { target: { value: 'not a url' } });
    await click('button', 'Extract');
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toBe('Entered URL is invalid');
    expect(alert.className).toContain('text-primary');
    expect(box.getAttribute('aria-describedby')).toBe(alert.id);
    expect(names()).not.toContain('prepare_url_import');
    expect(names()).not.toContain('start_url_import');
  });

  it('a valid URL calls prepare then start and shows the progress bar; Enter works too', async () => {
    const box = await toUrlBox();
    await fireEvent.input(box, { target: { value: `  ${URL_OK} ` } });
    await fireEvent.keyDown(box, { key: 'Enter' });
    await waitFor(() => expect(names()).toContain('start_url_import'));
    expect(names().filter((n) => n.endsWith('url_import'))).toEqual(['prepare_url_import', 'start_url_import']);
    expect(callsOf('prepare_url_import')[0]).toEqual({ url: URL_OK });
    expect(callsOf('start_url_import')[0]).toMatchObject({ url: URL_OK, resume: false });
    expect(await screen.findByText('Download in progress')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: 'Download in progress' })).toBeTruthy();
    expect((screen.getByRole('textbox', { name: 'enter url' }) as HTMLInputElement).disabled).toBe(true);
  });

  it('download events move the bar; an unknown total is indeterminate', async () => {
    await startDownload();
    const bar = () => screen.getByRole('progressbar', { name: 'Download in progress' });
    await emit({ phase: 'downloading', downloaded: 1048576, total: null });
    expect(bar().getAttribute('aria-valuenow')).toBeNull();
    expect(screen.getByTestId('download-text').textContent).toBe('1.0 MB');
    await emit({ phase: 'downloading', downloaded: 2097152, total: 8388608 });
    expect(bar().getAttribute('aria-valuenow')).toBe('25');
    expect(screen.getByTestId('download-text').textContent).toContain('25%');
    await emit({ phase: 'downloading', downloaded: 8388608, total: 8388608 });
    expect(bar().getAttribute('aria-valuenow')).toBe('100');
  });

  it('a failed download (403) shows the text under the box and returns to the source page', async () => {
    await startDownload();
    await emit({ phase: 'failed', stage: 'download', message: 'Error 403 when attempting download', http_status: 403 });
    expect(alertText()).toBe('Error 403 when attempting download');
    expect(screen.queryByRole('progressbar')).toBeNull();
    expect((screen.getByRole('textbox', { name: 'enter url' }) as HTMLInputElement).value).toBe(URL_OK);
  });

  it('Cancel during the download calls cancel_import; the cancelled event returns to the source page', async () => {
    await startDownload();
    await screen.findByRole('progressbar', { name: 'Download in progress' });
    await click('button', 'Cancel');
    await waitFor(() => expect(callsOf('cancel_import')).toEqual([{ job: 'job-1' }]));
    await emit({ phase: 'cancelled', back_to: 'source' });
    expect(screen.queryByRole('progressbar')).toBeNull();
    expect(screen.getByRole('button', { name: 'Extract' })).toBeTruthy();
  });

  it('a partial download prompts; Resume Download and Start Over pass resume true / false', async () => {
    for (const [button, resume] of [['Resume Download', true], ['Start Over', false]] as const) {
      cleanup();
      calls = [];
      resetImport();
      prep = { status: 'partial', message: 'Incomplete download file from the same URL found', partial_bytes: 5000 };
      const box = await toUrlBox();
      await fireEvent.input(box, { target: { value: URL_OK } });
      await click('button', 'Extract');
      expect(await screen.findByText('Incomplete download file from the same URL found')).toBeTruthy();
      expect(names()).not.toContain('start_url_import');
      await click('button', button);
      await waitFor(() => expect(callsOf('start_url_import')).toHaveLength(1));
      expect(callsOf('start_url_import')[0]).toMatchObject({ url: URL_OK, resume });
      expect(screen.queryByText('Incomplete download file from the same URL found')).toBeNull();
    }
  });

  it('a busy / invalid answer from Rust is shown under the box', async () => {
    prep = { status: 'busy', message: 'An import is already running', partial_bytes: 0 };
    const box = await toUrlBox();
    await fireEvent.input(box, { target: { value: URL_OK } });
    await click('button', 'Extract');
    expect(await screen.findByRole('alert')).toBeTruthy();
    expect(alertText()).toBe('An import is already running');
    expect(names()).not.toContain('start_url_import');
  });

  it.each([
    ['Local Audio File', 'audio'],
    ['Local Video File', 'video'],
  ])('%s shows Browse, which opens the file dialog for %s', async (label, kind) => {
    await toSource();
    await click('radio', label);
    expect(screen.queryByRole('textbox', { name: 'enter url' })).toBeNull();
    await click('button', 'Browse');
    await waitFor(() => expect(callsOf('import_file')).toHaveLength(1));
    expect(callsOf('import_file')[0]).toMatchObject({ kind });
    // dialog cancelled -> still on the source page, nothing else happens
    expect(screen.getByRole('button', { name: 'Browse' })).toBeTruthy();
    expect(alertText()).toBeNull();
  });

  it('a file without an audio track shows the message under Browse', async () => {
    fileResult = snap({ source: { kind: 'video-file', label: 'no-audio.mp4' }, phase: 'preparing' });
    await toSource();
    await click('radio', 'Local Video File');
    await click('button', 'Browse');
    expect(await screen.findByText('Preparing audio...')).toBeTruthy();
    await emit({ phase: 'failed', stage: 'prepare', message: 'Selected file no-audio.mp4 has no audio track', http_status: null });
    expect(alertText()).toBe('Selected file no-audio.mp4 has no audio track');
    expect(screen.getByRole('button', { name: 'Browse' })).toBeTruthy();
  });

  it('a track over 15 minutes shows the limit message', async () => {
    fileResult = snap({ source: { kind: 'audio-file', label: 'long.flac' }, phase: 'preparing' });
    await toSource();
    await click('radio', 'Local Audio File');
    await click('button', 'Browse');
    await emit({ phase: 'failed', stage: 'prepare', message: 'Tracks longer than 15 minutes are not supported', http_status: null });
    expect(alertText()).toBe('Tracks longer than 15 minutes are not supported');
  });

  it('a file import that goes straight to ready shows the edit pane', async () => {
    fileResult = snap({ source: { kind: 'audio-file', label: 'a.mp3' }, phase: 'ready', metadata: META, duration_s: 61 });
    await toSource();
    await click('radio', 'Local Audio File');
    await click('button', 'Browse');
    expect((await screen.findByRole('textbox', { name: 'Title' }) as HTMLInputElement).value).toBe('Slow Burn');
    expect(screen.getByTestId('import-source').textContent).toBe('a.mp3');
  });
});

describe('Import view: edit pane and extraction', () => {
  it('ready shows the pane in edit mode with every value in its field', async () => {
    await toEditPane();
    expect(field('Band').value).toBe('Amber Fields');
    expect(field('Album').value).toBe('Northern Roads');
    expect(field('Title').value).toBe('Slow Burn');
    expect(field('Composers').value).toBe('Ann Example\nBo Sample');
    expect(field('Year').value).toBe('2019');
    expect(field('Source link').value).toBe(URL_OK);
    expect(field('Copyright').value).toBe('');
    expect(field('Title').readOnly).toBe(false);
    expect(screen.getByTestId('import-source').textContent).toBe(URL_OK);
    expect(screen.getByText('3:33')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Extract' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeTruthy();
    expect(screen.queryByRole('radio')).toBeNull();
  });

  it('notes whether the original mix is kept, with a link to Settings', async () => {
    await toEditPane();
    await waitFor(() => expect(screen.getByText(/The original mix will not be kept/)).toBeTruthy());
    await click('button', 'Change in Settings');
    expect(ui.view).toBe('settings');
  });

  it('edits + Extract send the edited TrackEdits', async () => {
    await toEditPane();
    await fireEvent.input(field('Title'), { target: { value: '  New Title ' } });
    await fireEvent.input(field('Composers'), { target: { value: 'X\n\nY\n' } });
    await fireEvent.input(field('Year'), { target: { value: '' } });
    await fireEvent.input(field('Copyright'), { target: { value: '(c) me' } });
    await click('button', 'Extract');
    await waitFor(() => expect(callsOf('start_stem_extraction')).toHaveLength(1));
    expect(callsOf('start_stem_extraction')[0]).toEqual({
      job: 'job-1',
      edits: {
        band: 'Amber Fields', album: 'Northern Roads', title: 'New Title', composers: ['X', 'Y'],
        year: null, source_url: URL_OK, copyright: '(c) me',
      },
    });
  });

  it('an empty title blocks Extract with the field error', async () => {
    await toEditPane();
    await fireEvent.input(field('Title'), { target: { value: '   ' } });
    await click('button', 'Extract');
    expect(screen.getByText('Title is required')).toBeTruthy();
    expect(field('Title').getAttribute('aria-invalid')).toBe('true');
    expect(names()).not.toContain('start_stem_extraction');
  });

  it('an error from start_stem_extraction is shown on the pane', async () => {
    await toEditPane();
    clearMocks();
    mockIPC((cmd) => {
      if (cmd === 'start_stem_extraction') throw new Error('The edge-AI server is not configured');
      return undefined;
    });
    await click('button', 'Extract');
    await waitFor(() => expect(alertText()).toContain('The edge-AI server is not configured'));
  });

  it('uploading, queued, working and receiving show their steps', async () => {
    await toEditPane();
    await click('button', 'Extract');
    await waitFor(() => expect(screen.getByText('Sending to the edge-AI server')).toBeTruthy());
    expect(screen.queryByRole('textbox', { name: 'Title' })).toBeNull();
    await emit({ phase: 'uploading', sent: 1048576, total: 4194304 });
    expect(screen.getByRole('progressbar', { name: 'Sending to the edge-AI server' }).getAttribute('aria-valuenow')).toBe('25');
    await emit({ phase: 'queued' });
    expect(screen.getByText('Waiting for the edge-AI server...')).toBeTruthy();
    expect(screen.queryByText('Working...')).toBeNull();
    await emit({ phase: 'working', progress: 0.5 });
    const status = screen.getByText(/Working\.\.\./).closest('[role="status"]')!;
    expect(status.querySelector('svg.animate-spin')).not.toBeNull();
    expect(status.textContent).toContain('50%');
    await emit({ phase: 'receiving', done: 3, total: 6 });
    expect(screen.getByText('Receiving stems 3 of 6')).toBeTruthy();
    expect(screen.getByRole('progressbar', { name: 'Receiving stems 3 of 6' }).getAttribute('aria-valuenow')).toBe('50');
    await emit({ phase: 'saving' });
    expect(screen.getByText('Saving the track')).toBeTruthy();
  });

  it('saved shows the done panel; Show in Library selects the track in the Library', async () => {
    await toEditPane();
    await click('button', 'Extract');
    await emit({ phase: 'saved', track: stemTrack });
    expect(screen.getByText('Saved Slow Burn with 6 stems.')).toBeTruthy();
    await waitFor(() => expect(names()).toContain('list_tracks'));
    await click('button', 'Show in Library');
    expect(ui.view).toBe('library');
    expect(lib.selectedId).toBe(stemTrack.id);
    expect(screen.getByRole('button', { name: 'Stem Extraction' })).toBeTruthy();
  });

  it('Import another returns to a fresh source page', async () => {
    await toEditPane();
    await click('button', 'Extract');
    await emit({ phase: 'saved', track: stemTrack });
    await click('button', 'Import another track');
    expect(screen.getByRole('radio', { name: 'URL' })).toBeTruthy();
    expect(imp.job).toBeNull();
  });

  it('Cancel while extracting calls cancel_import and the cancelled event restores the edit pane with the edits', async () => {
    await toEditPane();
    await fireEvent.input(field('Title'), { target: { value: 'Edited' } });
    await click('button', 'Extract');
    await emit({ phase: 'working', progress: null });
    await click('button', 'Cancel');
    await waitFor(() => expect(callsOf('cancel_import')).toEqual([{ job: 'job-1' }]));
    await emit({ phase: 'cancelled', back_to: 'edit' });
    expect(field('Title').value).toBe('Edited');
    expect(field('Band').value).toBe('Amber Fields');
    expect(screen.getByRole('button', { name: 'Extract' })).toBeTruthy();
  });

  it('a failed extraction keeps the edit pane, shows the message and allows another try', async () => {
    await toEditPane();
    await fireEvent.input(field('Title'), { target: { value: 'Edited' } });
    await click('button', 'Extract');
    await emit({ phase: 'failed', stage: 'server', message: 'The edge-AI server said 500: boom', http_status: 500 });
    expect(alertText()).toBe('The edge-AI server said 500: boom');
    expect(field('Title').value).toBe('Edited');
    await click('button', 'Extract');
    await waitFor(() => expect(callsOf('start_stem_extraction')).toHaveLength(2));
  });

  it('Cancel on the edit pane asks first; No keeps the pane, Yes calls discard_import', async () => {
    await toEditPane();
    await click('button', 'Cancel');
    const dialog = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dialog).getByRole('button', { name: 'No' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(names()).not.toContain('discard_import');
    expect(field('Title').value).toBe('Slow Burn');

    await fireEvent.keyDown(window, { key: 'Escape' });
    const dialog2 = await screen.findByRole('alertdialog');
    await fireEvent.click(within(dialog2).getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(callsOf('discard_import')).toEqual([{ job: 'job-1' }]));
    await waitFor(() => expect(screen.getByRole('radio', { name: 'URL' })).toBeTruthy());
  });
});

describe('Import view: re-attach and footer', () => {
  it('re-mounting during a download re-attaches to the running job', async () => {
    watchResult = snap({ downloaded: 2097152, total: 8388608 });
    render(ImportView);
    expect(await screen.findByRole('progressbar', { name: 'Download in progress' })).toBeTruthy();
    expect(callsOf('watch_import')).toHaveLength(1);
    await emit({ phase: 'downloading', downloaded: 4194304, total: 8388608 });
    expect(screen.getByRole('progressbar', { name: 'Download in progress' }).getAttribute('aria-valuenow')).toBe('50');
  });

  it('re-mounting while the job waits for edits shows the edit pane', async () => {
    watchResult = snap({ phase: 'ready', metadata: META, duration_s: 10 });
    render(ImportView);
    expect((await screen.findByRole('textbox', { name: 'Title' }) as HTMLInputElement).value).toBe('Slow Burn');
  });

  it('re-mounting during the server step shows Working...', async () => {
    watchResult = snap({ phase: 'working', metadata: META });
    render(ImportView);
    expect(await screen.findByText(/Working\.\.\./)).toBeTruthy();
  });

  it('with no job the view stays on the menu', async () => {
    render(ImportView);
    await waitFor(() => expect(callsOf('watch_import')).toHaveLength(1));
    expect(screen.getByRole('button', { name: 'Stem Extraction' })).toBeTruthy();
  });

  it('the footer text follows the phase', async () => {
    render(StatusFooter);
    const footer = () => screen.getByRole('contentinfo').textContent!;
    expect(footer()).toContain('Ready');
    imp.job = snap({ phase: 'downloading' });
    await waitFor(() => expect(footer()).toContain('Download in progress'));
    imp.job = snap({ phase: 'queued' });
    await waitFor(() => expect(footer()).toContain('Waiting for the edge-AI server...'));
    imp.job = snap({ phase: 'working' });
    await waitFor(() => expect(footer()).toContain('Working...'));
    imp.job = null;
    await waitFor(() => expect(footer()).toMatch(/^Ready/));
  });
});
