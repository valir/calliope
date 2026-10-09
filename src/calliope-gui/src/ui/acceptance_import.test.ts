// QA acceptance tests for specs/gui-stem-extraction.md (frontend side). Written from the spec's
// acceptance criteria, through the rendered UI with the Tauri IPC mocked at the command level.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import ImportView from './views/ImportView.svelte';
import LibraryView from './views/LibraryView.svelte';
import { resetImport } from '$lib/import-state.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { checkUrl } from '$lib/url-check';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { ImportEvent, JobSnapshot, TrackEdits, TrackRecord, UrlPrep } from '$lib/ipc';

type Call = [string, Record<string, unknown>];
let calls: Call[] = [];
let channel: ((e: ImportEvent) => void) | null = null;
let prep: UrlPrep = { status: 'ready', message: '', partial_bytes: 0 };
let fileResult: JobSnapshot | null = null;
let tracks: TrackRecord[] = FIXTURE_TRACKS;

const URL_OK = 'https://media.example/watch?v=ok';
const META: TrackEdits = {
  band: 'Amber Fields', album: 'Northern Roads', title: 'Slow Burn', composers: ['Ann Example'],
  year: 2019, source_url: URL_OK, copyright: null,
};
const snap = (over: Partial<JobSnapshot> = {}): JobSnapshot => ({
  job: 'job-1', source: { kind: 'url', label: URL_OK }, phase: 'downloading', downloaded: 0, total: null,
  sent: 0, stems_done: 0, stems_total: 0, progress: null, duration_s: null, metadata: null, error: null,
  track: null, dropped: [], ...over,
});
const emit = (e: ImportEvent) => act(() => channel!(e));
const names = () => calls.map((c) => c[0]);

beforeEach(() => {
  calls = [];
  channel = null;
  prep = { status: 'ready', message: '', partial_bytes: 0 };
  fileResult = null;
  tracks = FIXTURE_TRACKS;
  resetImport();
  resetLibrary();
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (a.events) channel = (a.events as { onmessage: (e: ImportEvent) => void }).onmessage;
    switch (cmd) {
      case 'prepare_url_import': return prep;
      case 'start_url_import': return snap();
      case 'import_file': return fileResult;
      case 'watch_import': return null;
      case 'start_stem_extraction': return snap({ phase: 'uploading', metadata: a.edits as TrackEdits });
      case 'get_settings': return { theme: 'dark', repository_root: null, edge_ai_url: null, keep_original: false };
      case 'get_repository': return { root: '/tmp/r', is_default: true, status: 'ok' };
      case 'list_tracks': return { root: '/tmp/r', tracks, problems: [] };
      default: return undefined;
    }
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

const click = (role: 'button' | 'radio', name: string) => fireEvent.click(screen.getByRole(role, { name }));
async function urlPage(): Promise<HTMLInputElement> {
  render(ImportView);
  await click('button', 'Stem Extraction');
  await click('radio', 'URL');
  return screen.getByRole('textbox', { name: 'enter url' }) as HTMLInputElement;
}
async function submitUrl(text: string): Promise<void> {
  const box = await urlPage();
  await fireEvent.input(box, { target: { value: text } });
  await click('button', 'Extract');
}
async function toEdit(meta: TrackEdits = META): Promise<void> {
  await submitUrl(URL_OK);
  await waitFor(() => expect(names()).toContain('start_url_import'));
  await emit({ phase: 'ready', metadata: meta, duration_s: 100 });
  await screen.findByRole('textbox', { name: 'Title' });
}

describe('AC: tab, button, source page', () => {
  it('Import tab shows a button labeled exactly "Stem Extraction"', () => {
    render(ImportView);
    const b = screen.getByRole('button', { name: 'Stem Extraction' });
    expect(b.textContent?.trim()).toBe('Stem Extraction');
  });
  it('clicking it switches to the select-source page; URL shows the text box; the others show Browse', async () => {
    render(ImportView);
    await click('button', 'Stem Extraction');
    expect(screen.getByRole('radio', { name: 'URL' })).toBeTruthy();
    await click('radio', 'URL');
    expect(screen.getByRole('textbox', { name: 'enter url' })).toBeTruthy();
    await click('radio', 'Local Audio File');
    expect(screen.queryByRole('textbox', { name: 'enter url' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Browse' })).toBeTruthy();
    await click('radio', 'Local Video File');
    expect(screen.getByRole('button', { name: 'Browse' })).toBeTruthy();
  });
});

describe('AC: URL check and errors', () => {
  it.each([
    '', '   ', 'not a url', 'example.com', 'ftp://media.example/x', 'file:///etc/passwd', 'javascript:alert(1)',
    'data:text/html,hi', 'http://', 'https://', 'http://user:pw@media.example/x', '-o /tmp/x', '--exec=id',
    'http://media.example/a b', 'http://media.example/\u0000', 'mailto:a@b.c',
    'https://' + 'a'.repeat(2100) + '.example/',
  ])('malformed %j -> "Entered URL is invalid", nothing is sent to Rust', async (bad) => {
    await submitUrl(bad);
    const alert = screen.getByRole('alert');
    expect(alert.textContent?.trim()).toBe('Entered URL is invalid');
    expect(alert.className).toMatch(/text-primary|accent/);
    expect(names()).not.toContain('start_url_import');
    expect(names()).not.toContain('prepare_url_import');
    expect(checkUrl(bad)).toBe('Entered URL is invalid');
  });
  it.each(['http://media.example/x', 'https://youtu.be/abc', 'https://media.example/watch?v=1&t=2#frag', 'HTTPS://MEDIA.EXAMPLE/'])(
    'valid %s starts the download with the progress bar',
    async (good) => {
      await submitUrl(good);
      await waitFor(() => expect(names()).toContain('start_url_import'));
      expect(await screen.findByRole('progressbar', { name: 'Download in progress' })).toBeTruthy();
      expect(screen.queryByRole('alert')).toBeNull();
    },
  );
  it('a URL with an embedded newline is invalid (the input control would strip it, so check the function)', () => {
    expect(checkUrl('http://media.example/\n--exec')).toBe('Entered URL is invalid');
  });
  it('a leading dash in a URL is never a valid URL (argv injection guard, UI level)', () => {
    for (const s of ['-oops', '--version', '-', '- http://a.example']) expect(checkUrl(s)).not.toBeNull();
  });
  it('download progress is shown and updates', async () => {
    await submitUrl(URL_OK);
    await waitFor(() => expect(names()).toContain('start_url_import'));
    await emit({ phase: 'downloading', downloaded: 1000, total: 4000 });
    const bar = screen.getByRole('progressbar', { name: 'Download in progress' });
    expect(bar.getAttribute('aria-valuenow')).toBe('25');
    await emit({ phase: 'downloading', downloaded: 3000, total: 4000 });
    expect(bar.getAttribute('aria-valuenow')).toBe('75');
  });
  it('HTTP error text appears in the label under the URL box, verbatim', async () => {
    await submitUrl(URL_OK);
    await waitFor(() => expect(names()).toContain('start_url_import'));
    await emit({ phase: 'failed', stage: 'download', message: 'Error 404 when attempting download', http_status: 404 });
    expect(screen.getByRole('alert').textContent?.trim()).toBe('Error 404 when attempting download');
  });
  it('the existing-partial prompt has the exact text and both buttons', async () => {
    prep = { status: 'partial', message: 'Incomplete download file from the same URL found', partial_bytes: 10 };
    await submitUrl(URL_OK);
    expect(await screen.findByText('Incomplete download file from the same URL found')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Resume Download' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Start Over' })).toBeTruthy();
    expect(names()).not.toContain('start_url_import');
  });
});

describe('AC: edit pane and extraction', () => {
  it('after the download the edit pane shows with every extracted field editable', async () => {
    await toEdit();
    for (const [label, value] of [['Band', 'Amber Fields'], ['Album', 'Northern Roads'], ['Title', 'Slow Burn'], ['Year', '2019']] as const) {
      const el = screen.getByLabelText(label) as HTMLInputElement;
      expect(el.value).toBe(value);
      expect(el.readOnly).toBe(false);
      expect(el.disabled).toBe(false);
    }
  });
  it('Extract on the edit pane starts the stem extraction with the edited values', async () => {
    await toEdit();
    await fireEvent.input(screen.getByLabelText('Album'), { target: { value: 'Edited Album' } });
    await click('button', 'Extract');
    await waitFor(() => expect(names()).toContain('start_stem_extraction'));
    const c = calls.find((x) => x[0] === 'start_stem_extraction')![1] as { job: string; edits: TrackEdits };
    expect(c.edits.album).toBe('Edited Album');
    expect(c.job).toBe('job-1');
  });
  it('"Working..." wheel appears only when the server confirms start (working), not while queued', async () => {
    await toEdit();
    await click('button', 'Extract');
    await emit({ phase: 'queued' });
    expect(screen.queryByText(/^Working\.\.\.$/)).toBeNull();
    await emit({ phase: 'working', progress: null });
    expect(screen.getByText(/Working\.\.\./)).toBeTruthy();
  });
  it('title/year validation: invalid values never reach Rust', async () => {
    await toEdit();
    await fireEvent.input(screen.getByLabelText('Title'), { target: { value: '' } });
    await click('button', 'Extract');
    expect(names()).not.toContain('start_stem_extraction');
    await fireEvent.input(screen.getByLabelText('Title'), { target: { value: 'ok' } });
    await fireEvent.input(screen.getByLabelText('Year'), { target: { value: '99999' } });
    await click('button', 'Extract');
    expect(names()).not.toContain('start_stem_extraction');
  });
  it('video file without an audio track: exact message', async () => {
    fileResult = snap({ source: { kind: 'video-file', label: 'clip.mkv' }, phase: 'preparing' });
    render(ImportView);
    await click('button', 'Stem Extraction');
    await click('radio', 'Local Video File');
    await click('button', 'Browse');
    await emit({ phase: 'failed', stage: 'prepare', message: 'Selected file clip.mkv has no audio track', http_status: null });
    expect(screen.getByRole('alert').textContent?.trim()).toBe('Selected file clip.mkv has no audio track');
  });
  it('audio file: metadata appears in the edit pane', async () => {
    fileResult = snap({ source: { kind: 'audio-file', label: 'a.mp3' }, phase: 'ready', metadata: META, duration_s: 60 });
    render(ImportView);
    await click('button', 'Stem Extraction');
    await click('radio', 'Local Audio File');
    await click('button', 'Browse');
    expect(((await screen.findByLabelText('Title')) as HTMLInputElement).value).toBe('Slow Burn');
  });
});

describe('hostile text is rendered as text', () => {
  const evil = '<img src=x onerror="window.__pwned=1"><script>window.__pwned=1</script>';
  it('metadata, source label and server/tool error messages cannot inject markup', async () => {
    await toEdit({ ...META, title: evil, band: evil, album: evil, copyright: evil, source_url: 'https://media.example/' + evil });
    expect((screen.getByLabelText('Title') as HTMLInputElement).value).toBe(evil);
    await click('button', 'Extract');
    await emit({ phase: 'failed', stage: 'server', message: 'The edge-AI server said 500: ' + evil, http_status: 500 });
    expect(document.querySelector('img[src="x"]')).toBeNull();
    expect(document.querySelector('script')).toBeNull();
    expect((window as unknown as { __pwned?: number }).__pwned).toBeUndefined();
    expect(screen.getByRole('alert').textContent).toContain('<img src=x');
  });
  it('a stem track with hostile names is shown as text in the Library, with the S badge before the label', async () => {
    const t: TrackRecord = {
      ...FIXTURE_TRACKS[0], id: '0199b0a0-0000-7000-8000-0000000000ff', type: 'stem', audio: null,
      band: 'Zed', album: 'Zulu', title: evil,
      stems: [{ name: 'vocals', file: 'stems/vocals.flac' }], stem_model: evil,
    };
    tracks = [t];
    render(LibraryView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    await fireEvent.click(screen.getByRole('button', { name: 'Expand all' }));
    const row = await waitFor(() => {
      const r = screen.getAllByRole('treeitem').find((i) => i.querySelector('[data-track-type="stem"]'));
      expect(r).toBeTruthy();
      return r!;
    });
    const badge = row.querySelector('[data-track-type="stem"]')!;
    expect(badge.textContent).toBe('S');
    expect(badge.getAttribute('aria-label')).toBe('Stem track');
    // the badge precedes the name
    const kids = [...row.children];
    expect(kids.indexOf(badge as Element)).toBeLessThan(kids.findIndex((k) => k.textContent?.includes('<img')));
    expect(document.querySelector('img[src="x"]')).toBeNull();
    expect(document.querySelector('script')).toBeNull();
  });
  it('backing tracks keep their own badge; stem and backing differ', async () => {
    render(LibraryView);
    await waitFor(() => expect(screen.queryAllByRole('treeitem').length).toBeGreaterThan(0));
    await fireEvent.click(screen.getByRole('button', { name: 'Expand all' }));
    await waitFor(() => expect(document.querySelectorAll('[data-track-type]').length).toBe(FIXTURE_TRACKS.length));
    const types = new Set([...document.querySelectorAll('[data-track-type]')].map((e) => e.textContent));
    expect(types).toEqual(new Set(['S', 'B']));
  });
});
