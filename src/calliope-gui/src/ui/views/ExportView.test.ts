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
let lastTarget = 'mp3';
let targets: Record<string, unknown>[];
let chooseResult: string | null | Error;
let startError: string | null;

const snapshot = (o: Partial<ExportSnapshot> = {}): ExportSnapshot => ({
  job: 'j1', id: TRACK.id, title: TRACK.title, target: 'mp3', path: '/out/a.mp3', phase: 'writing',
  progress: 0, message: null, clipped: 0, info_error: null, ...o,
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
  targets = [];
  chooseResult = null;
  startError = null;
  sendIndex = 0;
  prepareError = null;
  plan = { token: 'tok', path: '/out/a.mp3', exists: false, base: '/out' };
  info = {
    id: TRACK.id, available: true, reason: null, source: 'backings/backing.flac',
    count_in: { bpm: 120, first_beat_ms: 0, lead_ms: 2000, skip_ms: 0 }, memory: 7,
    last: { rc5: null, mp3: null, ogg: null },
  };
  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    calls.push([cmd, a]);
    if (cmd === 'get_repository') return { root: '/tmp/repo', is_default: true, status: 'ok' };
    if (cmd === 'list_tracks') return { root: '/tmp/repo', tracks: structuredClone(FIXTURE_TRACKS), problems: [] };
    if (cmd === 'export_info') return { ...info, id: a.id };
    if (cmd === 'prepare_export') {
      targets.push(a.target as Record<string, unknown>);
      if (prepareError) throw prepareError;
      lastTarget = (a.target as { kind: string }).kind;
      return plan;
    }
    if (cmd === 'start_export') {
      if (startError) throw startError;
      channelId = (a.events as { id: number }).id;
      sendIndex = 0;
      return snapshot({ target: (lastTarget as 'rc5' | 'mp3' | 'ogg') });
    }
    if (cmd === 'choose_rc5_folder') {
      if (chooseResult instanceof Error) throw chooseResult.message;
      return chooseResult;
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
      .toBe('Count-in: 4 beats at 120 BPM before the first beat (adds 2.0 s)');
    expect(reason()).toBe('');
    expect((screen.getByLabelText('Memory') as HTMLInputElement).value).toBe('7');
    expect((screen.getByLabelText('Title') as HTMLInputElement).value).toBe(TRACK.title);
  });

  it('a track selected outside the tree replaces a stale band focus', async () => {
    lib.treeFocus = 'b:somebody';
    selectTrack(TRACK.id);
    render(ExportView);
    await waitFor(() => expect(btn('MP3 file').disabled).toBe(false));
    expect(reason()).toBe('');
    expect(lib.treeFocus).toBeNull();
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
    expect(c[1]).toEqual({ id: TRACK.id, target: { kind: 'rc5', memory: '7', reuse: null } });
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
    plan = { token: 'tok', path: '/out/a.mp3', exists: true, base: '/out' };
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
    send({ phase: 'done', path: '/out/a.mp3', clipped: 0, root: '/tmp/repo', records: [], info_error: null });
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

  it('sends an edited memory text as typed (Rust refuses 0, 100, abc) and never sends a path', async () => {
    plan = null;
    await renderWithTrack();
    const box = screen.getByLabelText('Memory') as HTMLInputElement;
    for (const text of ['0', '100', 'abc']) {
      calls.length = 0;
      await fireEvent.input(box, { target: { value: text } });
      await fireEvent.click(btn('to BOSS RC-5'));
      await waitFor(() => expect(names()).toContain('prepare_export'));
      expect(calls.find((x) => x[0] === 'prepare_export')![1]).toEqual({ id: TRACK.id, target: { kind: 'rc5', memory: text, reuse: null } });
      await tick();
    }
    plan = { token: 'tok', path: '/out/a.ogg', exists: false, base: '/out' };
    calls.length = 0;
    await fireEvent.click(btn('OGG file'));
    await waitFor(() => expect(names()).toContain('start_export'));
    expect(Object.keys(calls.find((x) => x[0] === 'start_export')![1]).sort()).toEqual(['events', 'overwrite', 'token']);
    expect(calls.find((x) => x[0] === 'prepare_export')![1]).toEqual({ id: TRACK.id, target: { kind: 'ogg' } });
  });

  // QA finding: exportTo() has no availability guard; with Svelte's delegated click a disabled button
  // (jsdom) still reaches it. Rust refuses anyway, and a real browser does not click a disabled button.
  it('asks nothing and sends nothing when the buttons are disabled', async () => {
    info = { ...info, available: false, reason: NO_TEMPO, count_in: null };
    selectTrack(TRACK.id);
    render(ExportView);
    await waitFor(() => expect(reason()).toBe(NO_TEMPO));
    for (const n of ['to BOSS RC-5', 'MP3 file', 'OGG file']) await fireEvent.click(btn(n));
    await tick();
    expect(names()).not.toContain('prepare_export');
  });

  it('declined confirmation by Escape or No never starts; the RC-5 done text says to eject', async () => {
    plan = { token: 'tok', path: '/rc5/ROLAND/WAVE/007_1/007_1.WAV', exists: true, base: '/rc5' };
    await renderWithTrack();
    await fireEvent.click(btn('to BOSS RC-5'));
    await screen.findByRole('alertdialog');
    expect(screen.getByRole('alertdialog').textContent).toContain('/rc5/ROLAND/WAVE/007_1/007_1.WAV');
    await fireEvent.click(screen.getByRole('button', { name: 'No' }));
    await tick();
    expect(names()).not.toContain('start_export');
    await fireEvent.click(btn('to BOSS RC-5'));
    await fireEvent.click(await screen.findByRole('button', { name: 'Yes' }));
    await waitFor(() => expect(names()).toContain('start_export'));
    send({ phase: 'done', path: plan.path, clipped: 0, root: '/tmp/repo', records: [], info_error: null });
    await waitFor(() => expect(screen.getByTestId('export-result').textContent)
      .toBe(`Written to ${plan!.path}. Eject the RC-5 before unplugging it.`));
  });

  it('shows the recorded memory per track, keeps typed text on a same-track reload', async () => {
    const other = FIXTURE_TRACKS[5];
    info = { ...info, memory: 7, last: { rc5: { memory: 7, at: '2026-10-24T18:30:00Z' }, mp3: null, ogg: null } };
    await renderWithTrack();
    const box = screen.getByLabelText('Memory') as HTMLInputElement;
    expect(box.value).toBe('7');
    await fireEvent.input(box, { target: { value: '12' } });
    info = { ...info, memory: 3, last: { rc5: null, mp3: null, ogg: null } };
    selectTrack(other.id);
    lib.treeFocus = `t:${other.id}`;
    await waitFor(() => expect(box.value).toBe('3'));
    // Back to the first track: its recorded memory replaces the text of the other one.
    info = { ...info, memory: 7 };
    selectTrack(TRACK.id);
    lib.treeFocus = `t:${TRACK.id}`;
    await waitFor(() => expect(box.value).toBe('7'));
    await fireEvent.input(box, { target: { value: '12' } });
    const before = names().filter((n) => n === 'export_info').length;
    const { loadInfo } = await import('$lib/export-state.svelte');
    await loadInfo(TRACK.id);
    expect(names().filter((n) => n === 'export_info').length).toBe(before + 1);
    expect(box.value).toBe('12');
  });

  it('shows the Last: lines', async () => {
    info = {
      ...info,
      last: {
        rc5: { memory: 7, at: '2026-10-24T12:00:00Z' },
        mp3: { path: '/m/A - B.mp3', at: '2026-10-24T12:00:00Z' },
        ogg: null,
      },
    };
    await renderWithTrack();
    await waitFor(() => expect(screen.getByTestId('export-last-rc5').textContent).toMatch(/^Last: memory 7, 2026-10-2\d$/));
    expect(screen.getByTestId('export-last-mp3').textContent).toMatch(/^Last: \/m\/A - B\.mp3, 2026-10-2\d$/);
    expect(screen.getByTestId('export-last-ogg').textContent).toBe('');
  });

  it('a done event for another track than the selected one leaves the shown info alone', async () => {
    const other = FIXTURE_TRACKS[5];
    await renderWithTrack();
    await fireEvent.click(btn('MP3 file'));
    await waitFor(() => expect(names()).toContain('start_export'));
    info = { ...info, memory: 3 };
    selectTrack(other.id);
    lib.treeFocus = `t:${other.id}`;
    await waitFor(() => expect((screen.getByLabelText('Memory') as HTMLInputElement).value).toBe('3'));
    info = { ...info, memory: 9 };
    send({ phase: 'done', path: '/out/a.mp3', clipped: 0, root: '/tmp/repo', records: [], info_error: null });
    await waitFor(() => expect(screen.getByTestId('export-result').textContent).toBe('Written to /out/a.mp3.'));
    await waitFor(() => expect(btn('MP3 file').disabled).toBe(false));
    expect(btn('to BOSS RC-5').disabled).toBe(false);
    expect(reason()).toBe('');
    expect((screen.getByLabelText('Memory') as HTMLInputElement).value).toBe('3');
    expect((screen.getByLabelText('Title') as HTMLInputElement).value).toBe(other.title);
  });

  it('a done event merges the records into the library and shows the info error', async () => {
    await renderWithTrack();
    await fireEvent.click(btn('MP3 file'));
    await waitFor(() => expect(names()).toContain('start_export'));
    const rec = {
      ...structuredClone(TRACK),
      exports: { rc5: null, mp3: { path: '/out/a.mp3', at: '2026-10-24T12:00:00Z' }, ogg: null },
    };
    lib.draft = null;
    send({
      phase: 'done', path: '/out/a.mp3', clipped: 0, root: '/tmp/repo', records: [rec],
      info_error: 'The export info wasn\'t saved: disk full',
    });
    await waitFor(() => expect(screen.getByTestId('export-info-error').textContent)
      .toBe('The export info wasn\'t saved: disk full'));
    expect(screen.getByTestId('export-result').textContent).toBe('Written to /out/a.mp3.');
    expect(lib.library!.tracks.find((t) => t.id === TRACK.id)!.exports!.mp3!.path).toBe('/out/a.mp3');
  });

  describe('quick RC-5 exports', () => {
    const rc5Plan = (base: string, exists = false): ExportPlan => ({
      token: 'tok', path: `${base}/ROLAND/WAVE/007_1/007_1.WAV`, exists, base,
    });
    const reuseOf = (i: number) => (targets[i] as { reuse?: string | null }).reuse;
    const row = () => screen.getByTestId('export-rc5-to');
    const folderText = () => screen.queryByTestId('export-rc5-folder')?.textContent ?? '';
    const change = () => screen.getByTestId('export-rc5-change') as HTMLButtonElement;

    /** One RC-5 click that starts a job, then the end of that job. */
    async function rc5Job(base: string, end: Record<string, unknown> = {}): Promise<void> {
      plan = rc5Plan(base);
      const n = names().filter((x) => x === 'start_export').length;
      await fireEvent.click(btn('to BOSS RC-5'));
      await waitFor(() => expect(names().filter((x) => x === 'start_export').length).toBe(n + 1));
      send({ phase: 'done', path: plan.path, clipped: 0, root: '/tmp/repo', records: [], info_error: null, ...end });
      await waitFor(() => expect(screen.getByTestId('export-result').textContent).not.toBe(''));
    }
    /** A click that only prepares: the plan asks to overwrite, which is declined. `base` null = no plan. */
    async function clickRc5(base: string | null = '/media/B'): Promise<void> {
      const n = targets.length;
      if (base !== null && !prepareError) plan = rc5Plan(base, true);
      await fireEvent.click(btn('to BOSS RC-5'));
      await waitFor(() => expect(targets.length).toBe(n + 1));
      if (base !== null && !prepareError) await fireEvent.click(await screen.findByRole('button', { name: 'No' }));
      await tick();
    }

    it('the first click sends no reuse and the row is empty but present', async () => {
      await renderWithTrack();
      await clickRc5();
      expect(reuseOf(0)).toBeNull();
      expect(row().textContent!.trim()).toBe('');
      expect(screen.queryByTestId('export-rc5-change')).toBeNull();
    });

    it('a finished export shows the folder and the next clicks reuse it, also for another track', async () => {
      await renderWithTrack();
      await rc5Job('/media/B');
      await waitFor(() => expect(folderText()).toBe('To: /media/B'));
      await clickRc5();
      expect(reuseOf(targets.length - 1)).toBe('/media/B');
      const other = FIXTURE_TRACKS[5];
      selectTrack(other.id);
      lib.treeFocus = `t:${other.id}`;
      await waitFor(() => expect((screen.getByLabelText('Title') as HTMLInputElement).value).toBe(other.title));
      await waitFor(() => expect(btn('to BOSS RC-5').disabled).toBe(false));
      await clickRc5();
      expect(reuseOf(targets.length - 1)).toBe('/media/B');
    });

    it('failed, cancelled and a cancelled dialog make the next click ask again', async () => {
      await renderWithTrack();
      for (const end of [{ phase: 'failed', message: 'boom' }, { phase: 'cancelled' }]) {
        await rc5Job('/media/B');
        await waitFor(() => expect(folderText()).toBe('To: /media/B'));
        await rc5Job('/media/B', end);
        await waitFor(() => expect(folderText()).toBe(''));
        await clickRc5();
        expect(reuseOf(targets.length - 1)).toBeNull();
      }
      await rc5Job('/media/B');
      await waitFor(() => expect(folderText()).toBe('To: /media/B'));
      plan = null;
      await clickRc5(null);
      expect(reuseOf(targets.length - 1)).toBe('/media/B');
      await waitFor(() => expect(folderText()).toBe(''));
      await clickRc5(null);
      expect(reuseOf(targets.length - 1)).toBeNull();
    });

    it('a start error ends the skip', async () => {
      await renderWithTrack();
      await rc5Job('/media/B');
      await waitFor(() => expect(folderText()).toBe('To: /media/B'));
      startError = 'cannot start';
      plan = rc5Plan('/media/B');
      await fireEvent.click(btn('to BOSS RC-5'));
      await waitFor(() => expect(folderText()).toBe(''));
    });

    it('a refusal and a declined overwrite keep the folder', async () => {
      await renderWithTrack();
      await rc5Job('/media/B');
      await waitFor(() => expect(folderText()).toBe('To: /media/B'));
      prepareError = 'Memory must be a number from 1 to 99';
      await clickRc5();
      await waitFor(() => expect(screen.getByTestId('export-result').textContent).toContain('Export failed'));
      expect(folderText()).toBe('To: /media/B');
      prepareError = null;
      await clickRc5();
      expect(reuseOf(targets.length - 1)).toBe('/media/B');
      expect(folderText()).toBe('To: /media/B');
      await clickRc5();
      expect(reuseOf(targets.length - 1)).toBe('/media/B');
    });

    it('leaving the view and coming back starts a new visit', async () => {
      await renderWithTrack();
      await rc5Job('/media/B');
      await waitFor(() => expect(folderText()).toBe('To: /media/B'));
      cleanup();
      await renderWithTrack();
      expect(folderText()).toBe('');
      expect(row()).toBeTruthy();
      await clickRc5();
      expect(reuseOf(targets.length - 1)).toBeNull();
    });

    it('Change... picks a folder, keeps it on cancel, shows a refusal, and is disabled while running', async () => {
      await renderWithTrack();
      await rc5Job('/media/B');
      await waitFor(() => expect(folderText()).toBe('To: /media/B'));

      chooseResult = '/media/C';
      await fireEvent.click(change());
      await waitFor(() => expect(folderText()).toBe('To: /media/C'));
      await clickRc5();
      expect(reuseOf(targets.length - 1)).toBe('/media/C');

      chooseResult = null;
      await fireEvent.click(change());
      await waitFor(() => expect(names().filter((n) => n === 'choose_rc5_folder').length).toBe(2));
      await tick();
      expect(folderText()).toBe('To: /media/C');

      chooseResult = new Error('No ROLAND folder in /x: is the RC-5 mounted?');
      await fireEvent.click(change());
      await waitFor(() => expect(screen.getByTestId('export-result').textContent)
        .toBe('Folder not changed: No ROLAND folder in /x: is the RC-5 mounted?'));
      expect(folderText()).toBe('To: /media/C');
      expect(screen.getByTestId('export-result').getAttribute('role')).toBe('alert');

      plan = { token: 'tok', path: '/out/a.mp3', exists: false, base: '/out' };
      await fireEvent.click(btn('MP3 file'));
      await waitFor(() => expect(names()).toContain('start_export'));
      send({ phase: 'writing', progress: 0.2 });
      await waitFor(() => expect(change().disabled).toBe(true));
    });

    it('MP3 after an RC-5 success sends no reuse and leaves the folder', async () => {
      await renderWithTrack();
      await rc5Job('/media/B');
      await waitFor(() => expect(folderText()).toBe('To: /media/B'));
      plan = { token: 'tok', path: '/out/a.mp3', exists: false, base: '/out' };
      await fireEvent.click(btn('MP3 file'));
      await waitFor(() => expect(names().filter((n) => n === 'start_export').length).toBe(2));
      send({ phase: 'done', path: '/out/a.mp3', clipped: 0, root: '/tmp/repo', records: [], info_error: null });
      await waitFor(() => expect(screen.getByTestId('export-result').textContent).toBe('Written to /out/a.mp3.'));
      expect(targets[targets.length - 1]).toEqual({ kind: 'mp3' });
      expect(folderText()).toBe('To: /media/B');
    });
  });
});
