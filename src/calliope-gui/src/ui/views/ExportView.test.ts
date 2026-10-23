import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import ExportView from './ExportView.svelte';
import { lib, resetLibrary, selectTrack } from '$lib/library-state.svelte';
import { resetExport } from '$lib/export-state.svelte';
import { FIXTURE_TRACKS } from '$lib/fixture-tracks';
import type { ExportInfo, ExportPlan, ExportSnapshot } from '$lib/ipc';

const TRACK = FIXTURE_TRACKS[6];
const NO_TEMPO = 'Set a tempo in the Player first: the export starts with a count-in';

let calls: [string, Record<string, unknown>][];
let info: ExportInfo;
let plan: ExportPlan | null;
let prepareError: string | null;
let channelId = 0;

const snapshot = (o: Partial<ExportSnapshot> = {}): ExportSnapshot => ({
  job: 'j1', id: TRACK.id, title: TRACK.title, target: 'mp3', path: '/out/a.mp3', phase: 'writing',
  progress: 0, message: null, clipped: 0, ...o,
});

function send(message: unknown): void {
  const internals = (window as unknown as {
    __TAURI_INTERNALS__: { runCallback: (id: number, data: unknown) => void };
  }).__TAURI_INTERNALS__;
  internals.runCallback(channelId, { index: sendIndex++, message });
}
let sendIndex = 0;

beforeEach(() => {
  resetLibrary();
  resetExport();
  calls = [];
  sendIndex = 0;
  prepareError = null;
  plan = { token: 'tok', path: '/out/a.mp3', exists: false };
  info = {
    id: TRACK.id, available: true, reason: null, source: 'backings/backing.flac',
    count_in: { bpm: 120, first_beat_ms: 0, lead_ms: 2000 }, memory: 7,
  };
  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'get_repository') return { root: '/tmp/repo', is_default: true, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/tmp/repo', tracks: structuredClone(FIXTURE_TRACKS), problems: [] };
    if (cmd === 'export_info') return { ...info, id: a.id };
    if (cmd === 'prepare_export') {
      if (prepareError) throw prepareError;
      return plan;
    }
    if (cmd === 'start_export') {
      channelId = (a.events as { id: number }).id;
      return snapshot();
    }
    if (cmd === 'cancel_export') return snapshot({ phase: 'cancelled' });
    return null;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

const btn = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const reason = () => screen.getByTestId('export-reason').textContent;
const names = () => calls.map((c) => c[0]);

async function renderWithTrack(): Promise<void> {
  selectTrack(TRACK.id);
  lib.treeFocus = `t:${TRACK.id}`;
  render(ExportView);
  await waitFor(() => expect(btn('MP3 file').disabled).toBe(false));
}

describe('Export view', () => {
  it('enables all three buttons for an available track and shows the source and count-in', async () => {
    await renderWithTrack();
    expect(btn('to BOSS RC-5').disabled).toBe(false);
    expect(btn('OGG file').disabled).toBe(false);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Export');
    expect(screen.getByText('Export...')).toBeTruthy();
    expect(screen.getByTestId('export-source').textContent).toBe('Backing track: backings/backing.flac');
    expect(screen.getByTestId('export-count-in').textContent)
      .toBe('Count-in: 4 beats at 120 BPM, ending on the first beat (adds 2.0 s)');
    expect(reason()).toBe('');
    expect((screen.getByLabelText('Memory') as HTMLInputElement).value).toBe('7');
    expect((screen.getByLabelText('Title') as HTMLInputElement).value).toBe(TRACK.title);
  });

  it('disables with the reason from export_info (no mix, no tempo)', async () => {
    info = { ...info, available: false, reason: 'No mix saved yet: save one in the Editor', source: null, count_in: null };
    selectTrack(TRACK.id);
    render(ExportView);
    await waitFor(() => expect(reason()).toBe('No mix saved yet: save one in the Editor'));
    expect(btn('MP3 file').disabled).toBe(true);
    expect(btn('to BOSS RC-5').disabled).toBe(true);
  });

  it('disables with the no-tempo reason', async () => {
    info = { ...info, available: false, reason: NO_TEMPO, count_in: null };
    selectTrack(TRACK.id);
    render(ExportView);
    await waitFor(() => expect(reason()).toBe(NO_TEMPO));
    expect(btn('OGG file').disabled).toBe(true);
    expect(screen.getByTestId('export-count-in').textContent).toBe('');
  });

  it('disables for a band or album row and with nothing selected', async () => {
    selectTrack(TRACK.id);
    lib.treeFocus = 'b:somebody';
    render(ExportView);
    await waitFor(() => expect(reason()).toBe('Select a track to export it'));
    expect(btn('MP3 file').disabled).toBe(true);
    lib.treeFocus = 'b:somebody|a:album';
    await tick();
    expect(reason()).toBe('Select a track to export it');
  });

  it('sends the memory text with the RC-5 target', async () => {
    await renderWithTrack();
    await fireEvent.click(btn('to BOSS RC-5'));
    await waitFor(() => expect(names()).toContain('prepare_export'));
    const c = calls.find((x) => x[0] === 'prepare_export')!;
    expect(c[1]).toEqual({ id: TRACK.id, target: { kind: 'rc5', memory: '7' } });
  });

  it('does nothing when the folder dialog is cancelled', async () => {
    plan = null;
    await renderWithTrack();
    await fireEvent.click(btn('MP3 file'));
    await waitFor(() => expect(names()).toContain('prepare_export'));
    await tick();
    expect(names()).not.toContain('start_export');
    expect(btn('MP3 file').disabled).toBe(false);
  });

  it('shows a refusal message', async () => {
    prepareError = 'No ROLAND folder in /x: is the RC-5 mounted?';
    await renderWithTrack();
    await fireEvent.click(btn('to BOSS RC-5'));
    await waitFor(() =>
      expect(screen.getByTestId('export-result').textContent).toBe('Export failed: No ROLAND folder in /x: is the RC-5 mounted?'));
    expect(names()).not.toContain('start_export');
  });

  it('asks before replacing: No writes nothing, Yes overwrites', async () => {
    plan = { token: 'tok', path: '/out/a.mp3', exists: true };
    await renderWithTrack();
    await fireEvent.click(btn('MP3 file'));
    const dlg = await screen.findByRole('alertdialog');
    expect(dlg.textContent).toContain('Replace the existing file /out/a.mp3? Its old content will be lost.');
    await fireEvent.click(screen.getByRole('button', { name: 'No' }));
    await tick();
    expect(names()).not.toContain('start_export');
    await fireEvent.click(btn('MP3 file'));
    await screen.findByRole('alertdialog');
    await fireEvent.click(screen.getByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(names()).toContain('start_export'));
    const s = calls.find((x) => x[0] === 'start_export')!;
    expect(s[1].overwrite).toBe(true);
    expect(s[1].token).toBe('tok');
  });

  it('shows progress and Cancel while writing, then the done text and reloads the info', async () => {
    await renderWithTrack();
    await fireEvent.click(btn('MP3 file'));
    await waitFor(() => expect(names()).toContain('start_export'));
    send({ phase: 'writing', progress: 0.42 });
    await waitFor(() => expect(screen.getByTestId('export-progress').textContent)
      .toBe(`Exporting ${TRACK.title} to /out/a.mp3 · 42 %`));
    expect(btn('MP3 file').disabled).toBe(true);
    expect(btn('to BOSS RC-5').disabled).toBe(true);
    expect(reason()).toBe('An export is running');
    await waitFor(() => expect(document.activeElement).toBe(btn('Cancel')));
    const infoCalls = names().filter((n) => n === 'export_info').length;
    info = { ...info, memory: 8 };
    send({ phase: 'done', path: '/out/a.mp3', clipped: 0 });
    await waitFor(() => expect(screen.getByTestId('export-result').textContent).toBe('Written to /out/a.mp3.'));
    await waitFor(() => expect(names().filter((n) => n === 'export_info').length).toBeGreaterThan(infoCalls));
    await waitFor(() => expect((screen.getByLabelText('Memory') as HTMLInputElement).value).toBe('8'));
    expect(btn('MP3 file').disabled).toBe(false);
  });

  it('Cancel sends cancel_export and shows the cancelled text', async () => {
    await renderWithTrack();
    await fireEvent.click(btn('OGG file'));
    await waitFor(() => expect(names()).toContain('start_export'));
    send({ phase: 'writing', progress: 0.1 });
    await fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(calls.find((x) => x[0] === 'cancel_export')![1]).toEqual({ job: 'j1' }));
    await waitFor(() => expect(screen.getByTestId('export-result').textContent)
      .toBe('Export cancelled; nothing was written.'));
  });

  it('shows a failure', async () => {
    await renderWithTrack();
    await fireEvent.click(btn('MP3 file'));
    await waitFor(() => expect(names()).toContain('start_export'));
    send({ phase: 'failed', message: 'Cannot write to /out: Permission denied (os error 13)' });
    await waitFor(() => expect(screen.getByTestId('export-result').textContent)
      .toBe('Export failed: Cannot write to /out: Permission denied (os error 13)'));
    expect(btn('MP3 file').disabled).toBe(false);
  });
});
