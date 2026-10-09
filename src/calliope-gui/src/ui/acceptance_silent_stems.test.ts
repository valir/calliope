// QA acceptance tests for the "drop silent stems" increment of specs/gui-stem-extraction.md
// (frontend side): the user is told what was dropped; an all-silent import is a clean failure.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import ImportView from './views/ImportView.svelte';
import { resetImport } from '$lib/import-state.svelte';
import { resetLibrary } from '$lib/library-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { ImportEvent, JobSnapshot, TrackEdits, TrackRecord } from '$lib/ipc';

type Call = [string, Record<string, unknown>];
let calls: Call[] = [];
let channel: ((e: ImportEvent) => void) | null = null;

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
const trackWith = (n: string[]): TrackRecord => ({
  ...FIXTURE_TRACKS[0], type: 'stem', audio: null, stem_model: 'htdemucs_6s',
  stems: n.map((x) => ({ name: x, file: `${x}.flac` })),
});
const emit = (e: ImportEvent) => act(() => channel!(e));
const logs = () => calls.filter((c) => c[0] === 'frontend_log').map((c) => c[1].message as string);

beforeEach(() => {
  calls = [];
  channel = null;
  resetImport();
  resetLibrary();
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    calls.push([cmd, a]);
    if (a.events) channel = (a.events as { onmessage: (e: ImportEvent) => void }).onmessage;
    switch (cmd) {
      case 'prepare_url_import': return { status: 'ready', message: '', partial_bytes: 0 };
      case 'start_url_import': return snap();
      case 'start_stem_extraction': return snap({ phase: 'uploading', metadata: a.edits as TrackEdits });
      case 'get_settings': return { theme: 'dark', repository_root: null, edge_ai_url: null, keep_original: false };
      case 'get_repository': return { root: '/tmp/r', is_default: true, status: 'ok' };
      case 'list_tracks': return { root: '/tmp/r', tracks: [], problems: [] };
      default: return undefined;
    }
  });
});
afterEach(() => { cleanup(); clearMocks(); });

async function toExtract(): Promise<void> {
  render(ImportView);
  await fireEvent.click(screen.getByRole('button', { name: 'Stem Extraction' }));
  await fireEvent.click(screen.getByRole('radio', { name: 'URL' }));
  const box = screen.getByRole('textbox', { name: 'enter url' });
  await fireEvent.input(box, { target: { value: URL_OK } });
  await fireEvent.click(screen.getByRole('button', { name: 'Extract' }));
  await waitFor(() => expect(calls.map((c) => c[0])).toContain('start_url_import'));
  await emit({ phase: 'preparing' });
  await emit({ phase: 'ready', metadata: META, duration_s: 213 });
  await screen.findByRole('textbox', { name: 'Title' });
  await fireEvent.click(screen.getByRole('button', { name: 'Extract' }));
}

describe('silent stems: what the user sees', () => {
  it('a 4-stem track with one dropped stem says 4 stems and names the dropped one', async () => {
    await toExtract();
    await emit({ phase: 'saved', track: trackWith(['vocals', 'drums', 'bass', 'guitar', 'other']), dropped: [{ name: 'piano', peak_dbfs: null }] });
    expect(screen.getByText('Saved Slow Burn with 5 stems.')).toBeTruthy();
    expect(screen.getByText('Dropped silent stems: piano.')).toBeTruthy();
    expect(logs().some((m) => m.includes('stems=5') && m.endsWith('dropped=piano'))).toBe(true);
  });

  it('the dropped line keeps the server order (not alphabetical)', async () => {
    await toExtract();
    await emit({
      phase: 'saved', track: trackWith(['vocals', 'drums', 'bass', 'guitar']),
      dropped: [{ name: 'piano', peak_dbfs: -60 }, { name: 'other', peak_dbfs: -70.2 }],
    });
    expect(screen.getByText('Dropped silent stems: piano, other.')).toBeTruthy();
  });

  it('an all-silent import fails with the message, returns to the edit pane and shows no Saved page', async () => {
    await toExtract();
    const msg = 'Every stem is silent (below -50 dBFS), so no track was saved';
    await emit({ phase: 'failed', stage: 'server', message: msg, http_status: null });
    expect(screen.getByRole('alert').textContent).toContain(msg);
    expect(screen.queryByText(/Saved Slow Burn/)).toBeNull();
    expect(screen.queryByText(/Dropped silent stems/)).toBeNull();
    expect(screen.getByRole('textbox', { name: 'Title' })).toBeTruthy();
  });
});
