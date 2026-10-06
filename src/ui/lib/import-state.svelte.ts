import {
  cancelImport,
  discardImport,
  frontendLog,
  importFile,
  prepareUrlImport,
  startStemExtraction,
  startUrlImport,
  watchImport,
  type ImportEvent,
  type ImportKind,
  type JobSnapshot,
} from './ipc';
import { buildTree } from './library-tree';
import { load, lib, selectTrack } from './library-state.svelte';
import { ui } from './app-state.svelte';
import { draftFromEdits, toEdits, validate, type DraftField, type DraftFields } from './track-draft';
import { checkUrl } from './url-check';

export type SourceChoice = 'url' | 'audio' | 'video';
export type Step = 'menu' | 'source' | 'edit' | 'extract' | 'done';

export const imp = $state({
  mode: 'menu' as 'menu' | 'stem',
  source: null as SourceChoice | null,
  url: '',
  /** The accent-colour label under the URL box / Browse button. */
  error: '',
  /** The "Incomplete download file" prompt (a partial download of this URL exists). */
  prompt: null as { bytes: number } | null,
  /** A request (prepare, start, file dialog) is in flight. */
  starting: false,
  job: null as JobSnapshot | null,
  draft: null as DraftFields | null,
  fieldErrors: {} as Partial<Record<DraftField, string>>,
  /** A failed extraction (server / save), shown on the edit pane. */
  extractError: '',
});

/** Counts events, so a command's returned snapshot never overwrites newer event data. */
let eventSeq = 0;

export function resetImport(): void {
  imp.mode = 'menu';
  imp.source = null;
  imp.url = '';
  imp.error = '';
  imp.prompt = null;
  imp.starting = false;
  imp.job = null;
  imp.draft = null;
  imp.fieldErrors = {};
  imp.extractError = '';
  eventSeq = 0;
}

const log = (m: string): void => void frontendLog(`import ${m}`);

export function step(): Step {
  if (imp.mode === 'menu') return 'menu';
  const j = imp.job;
  if (!j) return 'source';
  switch (j.phase) {
    case 'downloading':
    case 'preparing':
    case 'cancelled':
      return 'source';
    case 'ready':
    case 'failed':
      return 'edit';
    case 'saved':
      return 'done';
    default:
      return 'extract';
  }
}

/** True while the Rust side may be working (the footer and the Back button use it). */
export function jobRunning(): boolean {
  const s = step();
  return s === 'extract' || (s === 'source' && imp.job !== null);
}

const PHASE_TEXT: Record<string, string> = {
  downloading: 'Download in progress',
  preparing: 'Preparing audio...',
  ready: 'Import: edit the track details',
  uploading: 'Sending to the edge-AI server',
  queued: 'Waiting for the edge-AI server...',
  working: 'Working...',
  receiving: 'Receiving stems',
  saving: 'Saving the track',
  saved: 'Import finished',
  failed: 'Import failed',
};

/** The footer's left text. */
export function footerText(): string {
  const j = imp.job;
  return j ? (PHASE_TEXT[j.phase] ?? 'Ready') : 'Ready';
}

function stubJob(): JobSnapshot {
  return {
    job: '', source: { kind: 'url', label: '' }, phase: 'downloading', downloaded: 0, total: null,
    sent: 0, stems_done: 0, stems_total: 0, progress: null, duration_s: null, metadata: null,
    error: null, track: null,
  };
}

function setDraftFrom(j: JobSnapshot): void {
  if (j.metadata) {
    imp.draft = draftFromEdits(j.metadata);
    imp.fieldErrors = {};
  }
}

export function applyEvent(e: ImportEvent): void {
  eventSeq++;
  if (!imp.job) imp.job = stubJob();
  const j = imp.job; // read back: the reactive proxy, not the plain object
  imp.mode = 'stem';
  log(`phase=${e.phase} job=${j.job}`);
  switch (e.phase) {
    case 'downloading':
      j.phase = 'downloading';
      j.downloaded = e.downloaded;
      j.total = e.total;
      break;
    case 'preparing':
      j.phase = 'preparing';
      break;
    case 'ready':
      j.phase = 'ready';
      j.metadata = e.metadata;
      j.duration_s = e.duration_s;
      j.error = null;
      imp.extractError = '';
      setDraftFrom(j);
      break;
    case 'uploading':
      j.phase = 'uploading';
      j.sent = e.sent;
      j.total = e.total;
      break;
    case 'queued':
      j.phase = 'queued';
      break;
    case 'working':
      j.phase = 'working';
      j.progress = e.progress;
      break;
    case 'receiving':
      j.phase = 'receiving';
      j.stems_done = e.done;
      j.stems_total = e.total;
      break;
    case 'saving':
      j.phase = 'saving';
      break;
    case 'saved':
      j.phase = 'saved';
      j.track = e.track;
      log(`saved id=${e.track.id} stems=${e.track.stems.length} original=${e.track.original !== null}`);
      void load();
      break;
    case 'failed':
      log(`error stage=${e.stage} message=${e.message}`);
      if (e.stage === 'download' || e.stage === 'prepare') {
        // Back to the source page; the job is over.
        imp.job = null;
        imp.error = e.message;
        imp.starting = false;
      } else {
        j.phase = 'failed';
        j.error = { stage: e.stage, message: e.message, http_status: e.http_status };
        imp.extractError = e.message;
      }
      break;
    case 'cancelled':
      if (e.back_to === 'edit') {
        j.phase = 'ready';
        j.sent = 0;
        j.progress = null;
        j.stems_done = 0;
        j.stems_total = 0;
        j.error = null;
        imp.extractError = '';
      } else {
        imp.job = null;
        imp.error = '';
      }
      break;
  }
}

/** Takes the snapshot a command returned unless events already moved the job on. */
function adopt(snap: JobSnapshot, seqBefore: number): void {
  imp.mode = 'stem';
  if (imp.job && imp.job.job === '') {
    imp.job.job = snap.job;
    imp.job.source = snap.source;
  } else if (eventSeq === seqBefore) {
    imp.job = snap;
    setDraftFrom(snap);
  }
}

export function openStemExtraction(): void {
  imp.mode = 'stem';
  log('mode=stem-extraction');
}

export function backToMenu(): void {
  if (imp.job || imp.starting) return;
  imp.mode = 'menu';
  imp.source = null;
  imp.error = '';
  imp.prompt = null;
}

export function chooseSource(s: SourceChoice): void {
  if (imp.starting || imp.job) return;
  imp.source = s;
  imp.error = '';
  imp.prompt = null;
  log(`source=${s}`);
}

async function begin(url: string, resume: boolean): Promise<void> {
  const before = eventSeq;
  try {
    adopt(await startUrlImport(url, resume, applyEvent), before);
  } catch (err) {
    imp.error = String(err);
  }
}

export async function extractUrl(): Promise<void> {
  if (imp.starting || imp.job) return;
  imp.error = '';
  imp.prompt = null;
  const bad = checkUrl(imp.url);
  if (bad) {
    imp.error = bad;
    return;
  }
  imp.starting = true;
  try {
    const prep = await prepareUrlImport(imp.url.trim());
    if (prep.status === 'partial') imp.prompt = { bytes: prep.partial_bytes };
    else if (prep.status !== 'ready') imp.error = prep.message;
    else await begin(imp.url.trim(), false);
  } catch (err) {
    imp.error = String(err);
  } finally {
    imp.starting = false;
  }
}

export async function answerPrompt(resume: boolean): Promise<void> {
  if (!imp.prompt || imp.starting) return;
  imp.prompt = null;
  imp.starting = true;
  try {
    await begin(imp.url.trim(), resume);
  } finally {
    imp.starting = false;
  }
}

export async function browse(kind: ImportKind): Promise<void> {
  if (imp.starting || imp.job) return;
  imp.error = '';
  imp.starting = true;
  const before = eventSeq;
  try {
    const snap = await importFile(kind, applyEvent);
    if (snap) adopt(snap, before);
  } catch (err) {
    imp.error = String(err);
  } finally {
    imp.starting = false;
  }
}

export function setDraftField(field: DraftField, value: string): void {
  if (imp.draft) imp.draft = { ...imp.draft, [field]: value };
}

export async function extract(): Promise<void> {
  const j = imp.job;
  const d = imp.draft;
  if (!j || !d || imp.starting || (j.phase !== 'ready' && j.phase !== 'failed')) return;
  const errors = validate(d);
  imp.fieldErrors = errors;
  if (Object.keys(errors).length > 0) return;
  imp.extractError = '';
  imp.starting = true;
  const before = eventSeq;
  try {
    const snap = await startStemExtraction(j.job, toEdits(d));
    if (eventSeq === before) imp.job = snap;
  } catch (err) {
    imp.extractError = String(err);
  } finally {
    imp.starting = false;
  }
}

/** Stops the running job (download or extraction); the outcome arrives as a `cancelled` event. */
export async function cancelRunning(): Promise<void> {
  const j = imp.job;
  if (!j) return;
  try {
    await cancelImport(j.job);
  } catch (err) {
    imp.error = String(err);
  }
}

/** The edit pane's Cancel after the user confirmed: forget the prepared job. */
export async function discard(): Promise<void> {
  const j = imp.job;
  if (!j) return;
  try {
    await discardImport(j.job);
    imp.job = null;
    imp.draft = null;
    imp.fieldErrors = {};
    imp.extractError = '';
    imp.error = '';
  } catch (err) {
    imp.extractError = String(err);
  }
}

/** After "Saved": ready for another track. */
export function importAnother(): void {
  imp.job = null;
  imp.draft = null;
  imp.fieldErrors = {};
  imp.extractError = '';
  imp.error = '';
  imp.url = '';
  imp.source = null;
}

export function showInLibrary(): void {
  const t = imp.job?.track;
  if (!t) return;
  const node = buildTree(lib.library?.tracks ?? []).find((b) => b.albums.some((a) => a.tracks.some((x) => x.id === t.id)));
  if (node) {
    lib.expanded[node.key] = true;
    const alb = node.albums.find((a) => a.tracks.some((x) => x.id === t.id));
    if (alb) lib.expanded[alb.key] = true;
  }
  lib.search = '';
  selectTrack(t.id);
  importAnother();
  imp.mode = 'menu';
  ui.view = 'library';
}

/** Re-attaches to a running or finished job when the view is mounted again. */
export async function attach(): Promise<void> {
  const before = eventSeq;
  try {
    const snap = await watchImport(applyEvent);
    if (!snap || eventSeq !== before) return;
    if (snap.phase === 'cancelled') return;
    imp.mode = 'stem';
    imp.job = snap;
    if (snap.metadata && !imp.draft) setDraftFrom(snap);
    if (snap.phase === 'failed' && snap.error) {
      if (snap.error.stage === 'download' || snap.error.stage === 'prepare') {
        imp.job = null;
        imp.error = snap.error.message;
      } else {
        imp.extractError = snap.error.message;
      }
    }
    if (snap.phase === 'saved') void load();
    log(`attached phase=${snap.phase} job=${snap.job}`);
  } catch (err) {
    log(`error watch_import: ${String(err)}`);
  }
}
