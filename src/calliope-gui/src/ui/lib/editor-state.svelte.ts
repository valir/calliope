import {
  cancelBackingSave,
  closeEditor,
  editorEndSolo,
  editorLanePlay,
  editorNudge,
  editorPause,
  editorPlay,
  editorSeek,
  editorSetStem,
  editorStop,
  frontendLog,
  openEditor,
  saveBacking,
  EDITOR_SUPERSEDED,
  type EditorEvent,
  type EditorSnapshot,
  type LaneState,
  type TrackRecord,
  type TransportState,
  watchEditor,
} from './ipc';
import { load, selectTrack } from './library-state.svelte';
import { formatPosition } from './time-format';

export type EditorStatus = 'inactive' | 'loading' | 'active' | 'error';
type Variant = EditorSnapshot['variant'];

const STOPPED: TransportState = {
  playing: false, position_ms: 0, resume_pending: false, solo: null, clipping: false,
};

export const ed = $state({
  status: 'inactive' as EditorStatus,
  trackId: null as string | null,
  /** Inactive reason or load error, shown inline. */
  message: '',
  loading: null as { done: number; total: number } | null,
  stems: [] as LaneState[],
  durationMs: 0,
  sampleRate: 0,
  transport: { ...STOPPED } as TransportState,
  variant: null as Variant | null,
  /** 0..1 while a save runs. */
  saving: null as number | null,
  saveMessage: '',
  saveError: '',
  /** Playback stopped by an audio error. */
  audioError: '',
});

const log = (m: string): void => void frontendLog(m);
const fail = (command: string, err: unknown): void => log(`error editor ${command}: ${String(err)}`);

/** Identifies the current open; late snapshots and events of an older one are dropped. */
let generation = 0;
/** A backend session may exist (so it has to be closed on a change). */
let sessionOpen = false;
let key: string | null | undefined = undefined;

function resetFields(): void {
  ed.message = '';
  ed.loading = null;
  ed.stems = [];
  ed.durationMs = 0;
  ed.sampleRate = 0;
  ed.transport = { ...STOPPED };
  ed.variant = null;
  ed.saving = null;
  ed.saveMessage = '';
  ed.saveError = '';
  ed.audioError = '';
  gainPending.clear();
  seekPending = null;
}

export function resetEditor(): void {
  generation++;
  sessionOpen = false;
  key = undefined;
  ed.status = 'inactive';
  ed.trackId = null;
  resetFields();
}

function inactive(id: string | null, message: string): void {
  ed.status = 'inactive';
  ed.trackId = null;
  resetFields();
  ed.message = message;
  log(`editor inactive id=${id ?? 'none'}`);
}

/** Called when the selected track changes (or its record is reloaded). */
export function syncSelection(track: TrackRecord | null): void {
  const k = track?.id ?? null;
  if (k === key) return;
  key = k;
  const gen = ++generation;
  const hadSession = sessionOpen;
  sessionOpen = false;
  // Playback stops on a track change: closing the session stops it.
  const closing = hadSession ? closeEditor().catch((e) => fail('close_editor', e)) : Promise.resolve();

  if (!track) {
    inactive(null, 'Select a track with stems to build its backing track.');
    return;
  }
  if (track.stems.length === 0) {
    inactive(track.id, `${track.title} has no stems.`);
    return;
  }
  const gone = track.stems.find((s) => track.missing.includes(s.file));
  if (gone) {
    inactive(track.id, `Stem file ${gone.file} is missing.`);
    return;
  }

  ed.status = 'loading';
  ed.trackId = track.id;
  resetFields();
  ed.loading = { done: 0, total: track.stems.length };
  void closing.then(async () => {
    if (gen !== generation) return;
    sessionOpen = true;
    try {
      const snap = await openEditor(track.id, (e) => onEvent(gen, e));
      if (gen !== generation) return;
      applySnapshot(snap);
      log(`editor active id=${snap.id} stems=${snap.stems.length}`);
    } catch (err) {
      if (gen !== generation || String(err).includes(EDITOR_SUPERSEDED)) return;
      ed.status = 'error';
      ed.loading = null;
      ed.message = `Could not open "${track.title}": ${String(err).replace(/^Error: /, '')}`;
      fail('open_editor', err);
    }
  });
}

function applySnapshot(s: EditorSnapshot): void {
  ed.status = 'active';
  ed.trackId = s.id;
  ed.loading = null;
  ed.message = '';
  ed.stems = s.stems.map((l) => ({ ...l }));
  ed.durationMs = s.duration_ms;
  ed.sampleRate = s.sample_rate;
  ed.transport = { ...s.transport };
  ed.variant = { ...s.variant };
  ed.saving = s.saving;
}

/** Re-attaches to an open backend session after a webview reload (nothing is selected yet then). */
export async function attachEditor(): Promise<void> {
  if (key !== undefined && key !== null) return;
  const gen = ++generation;
  try {
    const snap = await watchEditor((e) => onEvent(gen, e));
    if (gen !== generation) return;
    if (!snap) { generation++; return; }
    key = snap.id;
    sessionOpen = true;
    applySnapshot(snap);
    selectTrack(snap.id);
    log(`editor active id=${snap.id} stems=${snap.stems.length}`);
  } catch (err) {
    fail('watch_editor', err);
  }
}

/** Stops playback when the view is left; the session and the mix stay. */
export function leaveEditor(): void {
  const id = ed.trackId;
  if (ed.status !== 'active' || !id) return;
  const t = ed.transport;
  if (!(t.playing || t.resume_pending || t.solo || t.position_ms > 0)) return;
  log('editor leave stop');
  const gen = generation;
  editorStop(id)
    .then((tr) => { if (gen === generation) ed.transport = tr; })
    .catch((e) => fail('editor_stop', e));
}

function onEvent(gen: number, e: EditorEvent): void {
  if (gen !== generation || e.id !== ed.trackId) return;
  switch (e.kind) {
    case 'loading':
      if (ed.status === 'loading') ed.loading = { done: e.done, total: e.total };
      break;
    case 'transport': {
      if (ed.status !== 'active') break;
      const { kind: _k, id: _i, ...t } = e;
      ed.transport = t;
      break;
    }
    case 'saving':
      ed.saving = e.progress;
      break;
    case 'saved': {
      ed.saving = null;
      ed.saveError = '';
      const secs = ed.sampleRate > 0 ? e.clipped_samples / (ed.sampleRate * 2) : 0;
      ed.saveMessage =
        e.clipped_samples > 0
          ? `Saved. ${e.clipped_samples} samples were clipped (about ${secs.toFixed(1)} s); lower some volumes and save again.`
          : 'Saved.';
      if (e.warnings.length > 0) ed.saveMessage += ` ${e.warnings.join(' ')}`;
      if (ed.variant) ed.variant = { ...ed.variant, file: e.file, exists: true };
      log(`editor saved id=${e.id} file=${e.file} clipped=${e.clipped_samples}`);
      void load();
      break;
    }
    case 'save-failed':
      ed.saving = null;
      ed.saveMessage = '';
      ed.saveError = e.message;
      break;
    case 'save-cancelled':
      ed.saving = null;
      ed.saveError = '';
      ed.saveMessage = 'Save cancelled.';
      break;
    case 'audio-error':
      ed.audioError = e.message;
      ed.transport.playing = false;
      break;
  }
}

// ---- derived ----

export const canPlay = (): boolean =>
  ed.status === 'active' && (ed.transport.playing || ed.stems.some((s) => s.unmuted));
export const stopEnabled = (): boolean =>
  ed.status === 'active' && (ed.transport.playing || ed.transport.position_ms > 0);
export const saveEnabled = (): boolean =>
  ed.status === 'active' && ed.saving === null && ed.stems.some((s) => s.unmuted);

// ---- actions ----

/** Runs a transport command; the result is dropped when the session changed meanwhile. */
async function transportCall(
  command: string,
  run: (id: string) => Promise<TransportState>,
  logLine?: (t: TransportState) => string,
): Promise<TransportState | null> {
  const id = ed.trackId;
  if (ed.status !== 'active' || !id) return null;
  const gen = generation;
  try {
    const t = await run(id);
    if (gen !== generation) return null;
    ed.transport = t;
    if (logLine) log(logLine(t));
    return t;
  } catch (err) {
    if (gen === generation) fail(command, err);
    return null;
  }
}

export async function togglePlay(): Promise<void> {
  if (ed.transport.playing) {
    await transportCall('editor_pause', editorPause, (t) => `editor pause position=${t.position_ms}`);
  } else {
    if (!canPlay()) return;
    await transportCall('editor_play', editorPlay, (t) => `editor play position=${t.position_ms}`);
  }
}

export async function stop(): Promise<void> {
  await transportCall('editor_stop', editorStop, (t) => `editor stop position=${t.position_ms}`);
}

/** Lane Play / Solo: checks the stem, solos it; a second press on the soloed stem ends the solo. */
export async function lanePlay(name: string): Promise<void> {
  const ending = ed.transport.solo === name;
  const t = await transportCall(
    'editor_lane_play',
    (id) => editorLanePlay(id, name),
    (r) => `editor solo name=${r.solo ?? 'none'}`,
  );
  if (t && t.solo === name && !ending) {
    const lane = ed.stems.find((l) => l.name === name);
    if (lane) lane.unmuted = true;
  }
}

export async function endSolo(): Promise<void> {
  await transportCall('editor_end_solo', editorEndSolo, (t) => `editor solo name=${t.solo ?? 'none'}`);
}

export async function nudge(deltaMs: number): Promise<void> {
  await transportCall('editor_nudge', (id) => editorNudge(id, deltaMs), () => `editor nudge delta=${deltaMs}`);
}

// Coalescing: at most one call in flight; while it runs only the latest value per key is kept.
const gainPending = new Map<string, { gain: number | null; unmuted: boolean }>();
let gainRunning = false;

async function drainGains(): Promise<void> {
  if (gainRunning) return;
  gainRunning = true;
  const gen = generation;
  try {
    while (gainPending.size > 0 && gen === generation) {
      const [name, v] = gainPending.entries().next().value!;
      gainPending.delete(name);
      const id = ed.trackId;
      if (!id) break;
      try {
        const lane = await editorSetStem(id, name, v.gain, v.unmuted);
        if (gen !== generation) break;
        log(`editor stem name=${name} gain=${v.gain ?? 'off'} unmuted=${v.unmuted}`);
        if (!gainPending.has(name)) {
          const l = ed.stems.find((s) => s.name === name);
          if (l) { l.gain_db = lane.gain_db; l.unmuted = lane.unmuted; }
        }
      } catch (err) {
        if (gen === generation) fail('editor_set_stem', err);
      }
    }
  } finally {
    gainRunning = false;
    if (gainPending.size > 0 && gen === generation) void drainGains();
  }
}

function setLane(name: string, change: Partial<Pick<LaneState, 'gain_db' | 'unmuted'>>): void {
  if (ed.status !== 'active') return;
  const lane = ed.stems.find((l) => l.name === name);
  if (!lane) return;
  Object.assign(lane, change);
  // Unchecking the soloed stem ends the solo (Rust confirms with a transport event).
  if (change.unmuted === false && ed.transport.solo === name) ed.transport.solo = null;
  gainPending.set(name, { gain: lane.gain_db, unmuted: lane.unmuted });
  void drainGains();
}

export const setGain = (name: string, db: number | null): void => setLane(name, { gain_db: db });
export const resetGain = (name: string): void => setLane(name, { gain_db: 0 });
export const setUnmuted = (name: string, unmuted: boolean): void => setLane(name, { unmuted });

let seekPending: number | null = null;
let seekRunning = false;

async function drainSeek(): Promise<void> {
  if (seekRunning) return;
  seekRunning = true;
  const gen = generation;
  try {
    while (seekPending !== null && gen === generation) {
      const pos = seekPending;
      seekPending = null;
      const id = ed.trackId;
      if (!id) break;
      try {
        const t = await editorSeek(id, pos);
        if (gen !== generation) break;
        if (seekPending === null) ed.transport = t;
        log(`editor seek position=${pos}`);
      } catch (err) {
        if (gen === generation) fail('editor_seek', err);
      }
    }
  } finally {
    seekRunning = false;
    if (seekPending !== null && gen === generation) void drainSeek();
  }
}

export function seek(positionMs: number): void {
  if (ed.status !== 'active') return;
  const p = Math.min(Math.max(0, Math.round(positionMs)), ed.durationMs);
  ed.transport.position_ms = p;
  seekPending = p;
  void drainSeek();
}

export async function save(): Promise<void> {
  const id = ed.trackId;
  if (!saveEnabled() || !id) return;
  const gen = generation;
  ed.saveMessage = '';
  ed.saveError = '';
  ed.audioError = '';
  ed.saving = 0;
  try {
    const s = await saveBacking(id);
    if (gen !== generation) return;
    // Events may already have finished the save; only take a running progress.
    if (ed.saving !== null) ed.saving = s.saving;
  } catch (err) {
    if (gen !== generation) return;
    ed.saving = null;
    ed.saveError = String(err).replace(/^Error: /, '');
    fail('save_backing', err);
  }
}

export async function cancelSave(): Promise<void> {
  const id = ed.trackId;
  if (!id || ed.saving === null) return;
  try {
    await cancelBackingSave(id);
  } catch (err) {
    fail('cancel_backing_save', err);
  }
}

/** For the pane's time display. */
export const positionText = (): string => formatPosition(ed.transport.position_ms);
