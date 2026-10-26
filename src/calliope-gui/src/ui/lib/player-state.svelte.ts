import {
  closePlayer,
  EDITOR_SUPERSEDED,
  frontendLog,
  openPlayer,
  playerLeave,
  playerLoop,
  playerNudge,
  playerPause,
  playerPlay,
  playerSeek,
  playerSelectSource,
  playerSkip,
  playerTempo,
  watchPlayer,
  type LoopAction,
  type PlayerEvent,
  type PlayerSnapshot,
  type PlayerTransport,
  type TrainingPlan,
  type TempoAction,
  type TempoSegment,
  type TrackRecord,
} from './ipc';
import { selectTrack } from './library-state.svelte';
import { scaleBpm } from './bpm-format';
import { MAX_SPEED, MIN_SPEED, parseSpeed, readoutText } from './speed-format';
import { cleanSectionName, parseOffset, sectionClicks, sectionCountIn } from './section-meta';
import { clampMoveSteps, sectionOffset } from './waveform-geometry';

export type PlayerStatus = 'inactive' | 'loading' | 'active' | 'error';

const STOPPED: PlayerTransport = {
  playing: false, counting_in: false, resume_pending: false, position_ms: 0,
  loop_playing: false, bpm: 0, played_bpm: 0, speed_pct: 100, tempo_index: null, training: null,
};

export const pl = $state({
  status: 'inactive' as PlayerStatus,
  trackId: null as string | null,
  /** Inactive reason or load error, shown inline. */
  message: '',
  /** 0..1 while loading. */
  progress: 0,
  /** Everything of the last snapshot except its transport. */
  snap: null as Omit<PlayerSnapshot, 'transport'> | null,
  transport: { ...STOPPED } as PlayerTransport,
  /** Refusal of the last tempo/loop/transport command. */
  actionError: '',
  /** Refusal of the last speed command (typed or from Rust); the next successful one clears it. */
  speedError: '',
  /** Playback stopped by an audio error. */
  audioError: '',
  /** player.json could not be written. */
  stateError: '',
  /** The section the user selected (index into the tempo segments), else the one at the playhead. */
  section: null as number | null,
  /** The BPM overlay: edit section `index`, or add a change at `addAt` ms. */
  bpmEdit: null as { index: number } | { addAt: number } | null,
  /** Refusal of the last section edit; the next successful edit clears it. */
  sectionError: '',
  /** A note under the section buttons (the found first beat); any other section action clears it. */
  sectionNote: '',
  /** The Find start call is running. */
  findingStart: false,
  /** The section whose name is being edited, captured on focus so the playhead can't move the edit. */
  nameEdit: null as { index: number; at: number; draft: string } | null,
  /** The section whose start offset field has the focus, captured on focus so the playhead can't move the edit. */
  offsetEdit: null as { index: number; at: number } | null,
  treeCollapsed: false,
  /** Bumped when the backend closed the session (the Editor took the track); the view reopens. */
  reopen: 0,
});

const log = (m: string): void => void frontendLog(m);
const fail = (command: string, err: unknown): void => log(`error player ${command}: ${String(err)}`);
const clean = (err: unknown): string => String(err).replace(/^Error: /, '');

let generation = 0;
let sessionOpen = false;
let key: string | null | undefined = undefined;
let lastReadout = '';
const OPEN_ATTEMPTS = 3;

function resetFields(): void {
  pl.message = '';
  pl.progress = 0;
  pl.snap = null;
  pl.transport = { ...STOPPED };
  pl.actionError = '';
  pl.speedError = '';
  lastReadout = '';
  pl.audioError = '';
  pl.stateError = '';
  pl.section = null;
  pl.bpmEdit = null;
  pl.sectionError = '';
  pl.sectionNote = '';
  pl.findingStart = false;
  pl.nameEdit = null;
  pl.offsetEdit = null;
  movePending = null;
  seekPending = null;
  seekGuard = null;
}

export function resetPlayer(): void {
  generation++;
  sessionOpen = false;
  key = undefined;
  pl.status = 'inactive';
  pl.trackId = null;
  resetFields();
}

function inactive(id: string | null, message: string): void {
  pl.status = 'inactive';
  pl.trackId = null;
  resetFields();
  pl.message = message;
  log(`player inactive id=${id ?? 'none'}`);
}

/** True when the record has at least one audio file that exists. */
export function playable(t: TrackRecord): boolean {
  const ok = (f: string | null): boolean => !!f && !t.missing.includes(f);
  return (
    ok(t.audio) || ok(t.original) || t.stems.some((s) => ok(s.file)) || t.backings.some((b) => ok(b.file))
  );
}

/** Called when the selected track changes. */
export function syncSelection(track: TrackRecord | null): void {
  const k = track?.id ?? null;
  if (k === key) return;
  key = k;
  const gen = ++generation;
  const hadSession = sessionOpen;
  sessionOpen = false;
  // A track change stops playback: closing the session saves the position and stops it.
  const closing = hadSession ? closePlayer().catch((e) => fail('close_player', e)) : Promise.resolve();

  if (!track) {
    inactive(null, 'Select a track to play it.');
    return;
  }
  if (!playable(track)) {
    inactive(track.id, `${track.title} has no audio to play.`);
    return;
  }

  pl.status = 'loading';
  pl.trackId = track.id;
  resetFields();
  void closing.then(async () => {
    sessionOpen = true;
    for (let attempt = 1; ; attempt++) {
      if (gen !== generation) return;
      try {
        const snap = await openPlayer(track.id, (e) => onEvent(gen, e));
        if (gen !== generation) return;
        applySnapshot(snap);
        logActive(snap);
        return;
      } catch (err) {
        if (gen !== generation) return;
        if (String(err).includes(EDITOR_SUPERSEDED) && attempt < OPEN_ATTEMPTS) {
          log(`player open superseded, retrying id=${track.id}`);
          continue;
        }
        pl.status = 'error';
        pl.message = `Could not open "${track.title}": ${clean(err)}`;
        fail('open_player', err);
        return;
      }
    }
  });
}

function logActive(s: PlayerSnapshot): void {
  log(
    `player active id=${s.id} source=${s.source} sources=${s.sources.length} mode=${s.mode} ` +
      `loops=${s.loops.length} position=${s.transport.position_ms} speed=${s.speed_pct}`,
  );
  logSections(s);
}

function logSections(s: { tempo: { segments: TempoSegment[] } }): void {
  const g = s.tempo.segments;
  log(
    `player sections segments=${g.length} bpm=${g.map((x) => x.bpm).join(',')} ` +
      `starts=${g.map((x) => x.start_ms).join(',')} ` +
      `metronome=${g.map((x) => (sectionClicks(x) ? 1 : 0)).join(',')} ` +
      `count_in=${g.map((x) => (sectionCountIn(x) ? 1 : 0)).join(',')} ` +
      `named=${g.filter((x) => (x.name ?? '') !== '').length}`,
  );
}

function applySnapshot(s: PlayerSnapshot): void {
  const { transport, ...rest } = s;
  const oldStarts = (pl.snap?.tempo.segments ?? []).map((g) => g.start_ms);
  pl.status = 'active';
  pl.trackId = s.id;
  pl.message = '';
  pl.snap = {
    ...rest,
    sources: rest.sources.map((x) => ({ ...x })),
    loops: rest.loops.map((l) => ({ ...l })),
    tempo: { ...rest.tempo, segments: rest.tempo.segments.map((g) => ({ ...g })) },
  };
  const sameStarts = oldStarts.length === s.tempo.segments.length && oldStarts.every((v, i) => v === s.tempo.segments[i].start_ms);
  if (sameStarts) dropSelectionOnMove(transport.tempo_index);
  pl.transport = { ...transport };
  logReadout();
  const n = s.tempo.segments.length;
  if (pl.section !== null && pl.section >= n) pl.section = n === 0 ? null : n - 1;
}

/** Re-attaches to an open backend session after a webview reload (nothing is selected yet then). */
export async function attachPlayer(): Promise<void> {
  if (key !== undefined && key !== null) return;
  const gen = ++generation;
  try {
    const snap = await watchPlayer((e) => onEvent(gen, e));
    if (gen !== generation) return;
    if (!snap) { generation++; return; }
    key = snap.id;
    sessionOpen = true;
    applySnapshot(snap);
    selectTrack(snap.id);
    logActive(snap);
  } catch (err) {
    fail('watch_player', err);
  }
}

/** Pauses when the view is left; the session stays. */
export function leavePlayer(): void {
  const id = pl.trackId;
  if (pl.status !== 'active' || !id) return;
  const t = pl.transport;
  if (!(t.playing || t.counting_in || t.resume_pending)) return;
  log('player leave pause');
  const gen = generation;
  playerLeave(id)
    .then((tr) => { if (gen === generation) setTransport(tr); })
    .catch((e) => fail('player_leave', e));
}

/** The playhead moved into another section (not onto the selected one): the selection is dropped. */
function dropSelectionOnMove(next: number | null): void {
  if (pl.section !== null && next !== pl.transport.tempo_index && next !== pl.section) pl.section = null;
}

/** Takes a transport; when the playhead moved into another section the selection is dropped, so the actions follow it. */
function setTransport(t: PlayerTransport): void {
  dropSelectionOnMove(t.tempo_index);
  pl.transport = t;
  logReadout();
}

function onEvent(gen: number, e: PlayerEvent): void {
  if (gen !== generation || e.id !== pl.trackId) return;
  switch (e.kind) {
    case 'loading':
      if (pl.status === 'loading') pl.progress = e.progress;
      break;
    case 'transport': {
      if (pl.status !== 'active') break;
      const { kind: _k, id: _i, ...t } = e;
      if (seekGuardActive()) {
        const g = seekGuard!;
        const reached = t.position_ms >= g.target && t.position_ms <= g.target + SEEK_SETTLE_MS;
        const moved = t.playing !== pl.transport.playing;
        if (moved || (!seekRunning && seekPending === null && reached)) seekGuard = null;
        else t.position_ms = pl.transport.position_ms;
      }
      setTransport(t);
      break;
    }
    case 'audio-error':
      pl.audioError = e.message;
      pl.transport.playing = false;
      pl.transport.counting_in = false;
      break;
    case 'state-error':
      pl.stateError = e.message;
      break;
    case 'closed':
      // The backend session is gone: forget the selection so the next sync opens the track again.
      key = null;
      sessionOpen = false;
      pl.reopen++;
      log(`player closed by the backend id=${e.id}`);
      break;
  }
}

/** Logs the Playing-BPM readout when its text changes. */
function logReadout(): void {
  const text = readoutText(pl.transport.played_bpm);
  if (text === lastReadout) return;
  lastReadout = text;
  log(`player readout bpm=${text === '' ? 'none' : text} speed=${pl.transport.speed_pct}`);
}

// ---- derived ----

export const playLabel = (): 'Play' | 'Pause' => (pl.transport.playing || pl.transport.counting_in ? 'Pause' : 'Play');
export const canPlay = (): boolean => pl.status === 'active';

// ---- actions ----

async function transportCall(
  command: string,
  run: (id: string) => Promise<PlayerTransport>,
  logLine?: (t: PlayerTransport) => string,
): Promise<void> {
  const id = pl.trackId;
  if (pl.status !== 'active' || !id) return;
  const gen = generation;
  try {
    const t = await run(id);
    if (gen !== generation) return;
    setTransport(t);
    pl.audioError = '';
    pl.actionError = '';
    pl.stateError = '';
    if (logLine) log(logLine(t));
  } catch (err) {
    if (gen !== generation) return;
    pl.audioError = clean(err);
    fail(command, err);
  }
}

/** Runs a command that returns a snapshot; the result is dropped when the session changed meanwhile. */
async function snapshotCall(
  command: string,
  run: (id: string) => Promise<PlayerSnapshot>,
  logLine: (s: PlayerSnapshot) => string,
  onError?: (message: string) => void,
): Promise<boolean> {
  const id = pl.trackId;
  if (pl.status !== 'active' || !id) return false;
  const gen = generation;
  try {
    const s = await run(id);
    if (gen !== generation) return false;
    applySnapshot(s);
    pl.actionError = '';
    pl.stateError = '';
    log(logLine(s));
    return true;
  } catch (err) {
    if (gen !== generation) return false;
    if (onError) onError(clean(err));
    else pl.actionError = clean(err);
    fail(command, err);
    return false;
  }
}

export async function togglePlay(): Promise<void> {
  if (pl.transport.playing || pl.transport.counting_in) {
    await transportCall('player_pause', playerPause, (t) => `player pause position=${t.position_ms}`);
  } else {
    await transportCall('player_play', playerPlay, (t) => `player play position=${t.position_ms}`);
  }
}

/** Stops playback (the position stays) and loads the source. */
export async function selectSource(source: string): Promise<void> {
  await snapshotCall('player_select_source', (id) => playerSelectSource(id, source), (s) => `player source key=${s.source}`);
}

export async function nudge(delta: number): Promise<void> {
  await transportCall('player_nudge', (id) => playerNudge(id, delta), () => `player seek position=${pl.transport.position_ms}`);
}

export async function skip(delta: number): Promise<void> {
  await transportCall('player_skip', (id) => playerSkip(id, delta), () => `player skip delta=${delta}`);
}

export async function setBpm(bpm: number): Promise<void> {
  await snapshotCall(
    'player_tempo',
    (id) => playerTempo(id, { kind: 'set-bpm', bpm }),
    (s) => `player bpm value=${bpm} position=${s.transport.position_ms}`,
  );
}

export async function loop(action: LoopAction): Promise<void> {
  const name = action.kind === 'mark' ? `mark-${action.edge}` : action.kind;
  await snapshotCall(
    'player_loop',
    (id) => playerLoop(id, action),
    (s) => {
      const cur = s.current_loop === null ? null : s.loops[s.current_loop];
      return `player loop ${name} index=${s.current_loop ?? 'none'} count=${s.loops.length} complete=${!!cur && cur.end !== null} end=${cur?.end_ms ?? 'none'}`;
    },
  );
}

// Seek coalescing: one call in flight; while it runs only the latest value is kept.
let seekPending: number | null = null;
let seekRunning = false;
let seekGuard: { target: number; until: number } | null = null;
const SEEK_SETTLE_MS = 1500;
const SEEK_GUARD_MS = 1000;

function seekGuardActive(): boolean {
  if (seekRunning || seekPending !== null) return true;
  if (seekGuard && Date.now() < seekGuard.until) return true;
  seekGuard = null;
  return false;
}

async function drainSeek(): Promise<void> {
  if (seekRunning) return;
  seekRunning = true;
  const gen = generation;
  try {
    while (seekPending !== null && gen === generation) {
      const pos = seekPending;
      seekPending = null;
      const id = pl.trackId;
      if (!id) break;
      try {
        const t = await playerSeek(id, pos);
        if (gen !== generation) break;
        if (seekPending === null) setTransport(t);
        log(`player seek position=${pos}`);
      } catch (err) {
        if (gen === generation) fail('player_seek', err);
      }
    }
  } finally {
    seekRunning = false;
    if (seekGuard) seekGuard.until = Date.now() + SEEK_GUARD_MS;
    if (seekPending !== null && gen === generation) void drainSeek();
  }
}

export function seek(positionMs: number): void {
  if (pl.status !== 'active' || !pl.snap) return;
  const p = Math.min(Math.max(0, Math.round(positionMs)), pl.snap.duration_ms);
  pl.transport.position_ms = p;
  seekPending = p;
  seekGuard = { target: p, until: Date.now() + SEEK_GUARD_MS };
  void drainSeek();
}

// ---- sections ----

/** The section the actions apply to: the selection (until the playhead moves into another section), else the one at the playhead. */
export function targetSection(): number | null {
  const n = pl.snap?.tempo.segments.length ?? 0;
  if (n === 0) return null;
  const i = pl.section ?? pl.transport.tempo_index;
  return i === null || i < 0 ? null : Math.min(i, n - 1);
}

export function selectSection(index: number | null): void {
  const n = pl.snap?.tempo.segments.length ?? 0;
  pl.section = index === null || n === 0 ? null : Math.min(Math.max(0, index), n - 1);
}

/** Runs a section edit; `after` sees the old and the new segment starts. Returns true when it was applied. */
async function sectionCall(
  kind: 'bpm' | 'scale' | 'add' | 'remove' | 'move' | 'flags' | 'name' | 'jump',
  index: number,
  action: (id: string) => Promise<PlayerSnapshot>,
  after?: (oldStarts: number[], s: PlayerSnapshot) => void,
  detail?: (s: PlayerSnapshot) => string,
): Promise<boolean> {
  const id = pl.trackId;
  if (pl.status !== 'active' || !id) return false;
  const gen = generation;
  const oldStarts = (pl.snap?.tempo.segments ?? []).map((g) => g.start_ms);
  try {
    const s = await action(id);
    if (gen !== generation) return false;
    applySnapshot(s);
    after?.(oldStarts, s);
    pl.sectionError = '';
    pl.sectionNote = '';
    pl.stateError = '';
    log(`player section ${kind} index=${index}${detail ? ' ' + detail(s) : ''}`);
    logSections(s);
    return true;
  } catch (err) {
    if (gen !== generation) return false;
    pl.sectionError = clean(err);
    fail('player_tempo', err);
    return false;
  }
}

const mmss = (ms: number): string => {
  const total = Math.max(0, ms) / 1000;
  const m = Math.floor(total / 60);
  const sec = (total - m * 60).toFixed(3);
  return `${m}:${Number(sec) < 10 ? '0' : ''}${sec}`;
};

/** Finds where the music starts and puts the first beat there; one call at a time, ignored after a track switch. */
export async function findStart(): Promise<void> {
  const id = pl.trackId;
  if (pl.status !== 'active' || !id || pl.findingStart) return;
  if ((pl.snap?.tempo.segments[0]?.bpm ?? 0) <= 0) return;
  const gen = generation;
  pl.findingStart = true;
  pl.sectionError = '';
  pl.sectionNote = '';
  log('player first-beat find');
  try {
    const s = await playerTempo(id, { kind: 'find-start' });
    if (gen !== generation) return;
    applySnapshot(s);
    const start = s.tempo.segments[0]?.start_ms ?? 0;
    pl.sectionNote = `First beat at ${mmss(start)}`;
    pl.stateError = '';
    log(`player first-beat start=${Math.round(start)}`);
    logSections(s);
  } catch (err) {
    if (gen !== generation) return;
    pl.sectionError = clean(err);
    fail('player_tempo', err);
  } finally {
    if (gen === generation) pl.findingStart = false;
  }
}

const startOf = (i: number): number | null => pl.snap?.tempo.segments[i]?.start_ms ?? null;

export function editSectionBpm(index: number): void {
  if (startOf(index) === null) return;
  pl.section = index;
  pl.bpmEdit = { index };
}

export function cancelSectionBpm(): void {
  pl.bpmEdit = null;
  pl.sectionError = '';
}

/** Opens the overlay for a new change at the playhead (captured now). */
export function startAdd(): void {
  if (pl.status !== 'active' || !pl.snap || pl.snap.tempo.segments.length === 0) return;
  pl.bpmEdit = { addAt: pl.transport.position_ms };
}

export async function commitSectionBpm(bpm: number): Promise<void> {
  const edit = pl.bpmEdit;
  if (!edit) return;
  pl.bpmEdit = null;
  if ('addAt' in edit) {
    await sectionCall('add', -1, (id) => playerTempo(id, { kind: 'add-change', position: edit.addAt, bpm }), (old, s) => {
      const now = s.tempo.segments;
      let i = 0;
      while (i < old.length && i < now.length && old[i] === now[i].start_ms) i++;
      pl.section = Math.min(i, now.length - 1);
    });
    return;
  }
  const at = startOf(edit.index);
  if (at === null) return;
  await sectionCall('bpm', edit.index, (id) => playerTempo(id, { kind: 'section-bpm', index: edit.index, at, bpm }));
}

export async function removeChange(): Promise<void> {
  const index = targetSection();
  const at = index === null ? null : startOf(index);
  if (index === null || at === null) return;
  await sectionCall('remove', index, (id) => playerTempo(id, { kind: 'remove-change', index, at }), () => {
    pl.section = Math.max(0, index - 1);
  });
}

export async function scaleSection(factor: 2 | 0.5): Promise<void> {
  const index = targetSection();
  const seg = index === null ? undefined : pl.snap?.tempo.segments[index];
  if (index === null || !seg || scaleBpm(seg.bpm, factor) === null) return;
  const at = seg.start_ms;
  await sectionCall('scale', index, (id) =>
    playerTempo(id, { kind: 'scale-section', index, at, factor: factor === 2 ? 'double' : 'halve' }),
  );
}

// Move coalescing: one call in flight; while it runs only the latest target is kept. `at` is captured when
// the move is requested, so an Add/Remove landing in between makes the queued move stale (the backend refuses
// it) instead of hitting another section. Only a queued move of the section whose move is in flight takes its
// `at` from the snapshot answering that move (null until then).
let movePending: { index: number; steps: number; at: number | null } | null = null;
let moveRunning: { index: number } | null = null;

async function drainMoves(): Promise<void> {
  if (moveRunning) return;
  const gen = generation;
  try {
    while (movePending !== null && gen === generation) {
      const { index, steps, at } = movePending;
      movePending = null;
      if (at === null) continue;
      moveRunning = { index };
      const ok = await sectionCall('move', index, (id) => playerTempo(id, { kind: 'move-change', index, at, steps }));
      moveRunning = null;
      if (ok && movePending !== null && (movePending as { index: number; at: number | null }).index === index) {
        (movePending as { at: number | null }).at ??= startOf(index);
      }
    }
  } finally {
    moveRunning = null;
    if (movePending !== null && gen === generation) void drainMoves();
  }
}

/** Moves change `index` to `steps` 1/64 beats from its reference; calls are coalesced. */
export function moveChange(index: number, steps: number): void {
  if (pl.status !== 'active') return;
  const same = moveRunning !== null && moveRunning.index === index;
  const at = movePending !== null && movePending.index === index ? movePending.at : same ? null : startOf(index);
  movePending = { index, steps, at };
  void drainMoves();
}

// ---- flags, names, jumps ----

/** Sets the metronome and count-in of the captured name-edit section, else the target section. */
export async function setSectionFlags(metronome: boolean, countIn: boolean): Promise<void> {
  const edit = pl.nameEdit;
  const index = edit ? edit.index : targetSection();
  const at = edit ? edit.at : index === null ? null : startOf(index);
  if (index === null || at === null) return;
  await sectionCall(
    'flags',
    index,
    (id) => playerTempo(id, { kind: 'section-flags', index, at, metronome, count_in: countIn }),
    undefined,
    () => `metronome=${metronome} count_in=${countIn}`,
  );
}

/** Captures the target section for a name edit (on focus). */
export function beginNameEdit(): void {
  if (pl.nameEdit || pl.status !== 'active') return;
  const index = targetSection();
  const seg = index === null ? undefined : pl.snap?.tempo.segments[index];
  if (index === null || !seg) return;
  pl.nameEdit = { index, at: seg.start_ms, draft: seg.name ?? '' };
}

/** Sends the cleaned draft; nothing is sent when the name is unchanged. */
export async function commitSectionName(): Promise<void> {
  const edit = pl.nameEdit;
  if (!edit) return;
  pl.nameEdit = null;
  const seg = pl.snap?.tempo.segments[edit.index];
  const name = cleanSectionName(edit.draft);
  if (!seg || seg.start_ms !== edit.at || name === (seg.name ?? '')) return;
  await sectionCall(
    'name',
    edit.index,
    (id) => playerTempo(id, { kind: 'section-name', index: edit.index, at: edit.at, name }),
    undefined,
    () => `length=${Array.from(name).length}`,
  );
}

/** Drops the edit; the box shows the stored name again. */
export function cancelSectionName(): void {
  pl.nameEdit = null;
}

/** Jumps to the start or the last frame (the end) of the target section. */
export async function jumpSection(edge: 'start' | 'end'): Promise<void> {
  const index = targetSection();
  const at = index === null ? null : startOf(index);
  if (index === null || at === null) return;
  await sectionCall(
    'jump',
    index,
    (id) => playerTempo(id, { kind: 'jump-section', index, at, edge }),
    undefined,
    (s) => `edge=${edge} position=${s.transport.position_ms}`,
  );
}

/** Goes to the start of the previous (-1) or next (+1) section, as |< does for the target section. */
export async function stepSection(dir: -1 | 1): Promise<void> {
  const from = targetSection();
  if (from === null) return;
  const to = from + dir;
  const at = startOf(to);
  if (at === null) return;
  await sectionCall(
    'jump',
    to,
    (id) => playerTempo(id, { kind: 'jump-section', index: to, at, edge: 'start' }),
    () => {
      pl.section = to;
    },
    (s) => `edge=start position=${s.transport.position_ms} via=${dir < 0 ? 'prev' : 'next'}`,
  );
}

// ---- start offset field ----

/** Captures the target section for an offset edit (on focus). */
export function beginOffsetEdit(): void {
  if (pl.offsetEdit || pl.status !== 'active') return;
  const index = targetSection();
  const at = index === null ? null : startOf(index);
  if (index === null || at === null) return;
  pl.offsetEdit = { index, at };
}

/** Drops the capture; the field follows the target section again. */
export function cancelOffsetEdit(): void {
  pl.offsetEdit = null;
}

/** Moves the captured (else target) section's start to `text` 1/64 beats from its reference, clamped like a drag. False when the text is refused. */
export async function commitSectionOffset(text: string): Promise<boolean> {
  const n = parseOffset(text);
  if (n === null) {
    pl.sectionError = 'Type a whole number of 1/64 beats, like +10 or -8';
    log(`player section offset refused text=${text}`);
    return false;
  }
  const segs = pl.snap?.tempo.segments;
  const edit = pl.offsetEdit;
  const index = edit ? edit.index : targetSection();
  const at = edit ? edit.at : index === null ? null : startOf(index);
  if (!segs || index === null || at === null || !segs[index]) return true;
  const steps = clampMoveSteps(segs, index, n, pl.snap!.duration_ms);
  if (steps === sectionOffset(segs, index).steps) {
    log(`player section offset index=${index} text=${text} steps=${steps} unchanged`);
    pl.sectionError = '';
    return true;
  }
  log(`player section offset index=${index} text=${text} steps=${steps}`);
  await sectionCall('move', index, (id) => playerTempo(id, { kind: 'move-change', index, at, steps }));
  return true;
}

// ---- speed and training ----

export type SpeedVia = 'minus' | 'plus' | 'reset' | 'typed' | 'key';

/** True while a speed-training run is active: the speed controls are locked. */
export const speedLocked = (): boolean => pl.transport.training !== null;

/** The speed to show: the heard speed during a run, else the track speed (the heard one lags a callback). */
export const shownSpeed = (): number => (speedLocked() ? pl.transport.speed_pct : (pl.snap?.speed_pct ?? pl.transport.speed_pct));

async function speedCall(action: TempoAction, via: SpeedVia): Promise<void> {
  if (pl.status !== 'active' || speedLocked()) return;
  const ok = await snapshotCall('player_tempo', (id) => playerTempo(id, action), (s) => `player speed percent=${s.speed_pct} via=${via}`, (e) => {
    pl.speedError = e;
  });
  if (ok) pl.speedError = '';
}

/** Changes the track speed by `steps` presets (-1 slower, +1 faster). */
export async function speedStep(steps: number, via: 'minus' | 'plus' | 'key'): Promise<void> {
  await speedCall({ kind: 'speed-step', steps }, via);
}

export async function speedReset(): Promise<void> {
  await speedCall({ kind: 'speed-reset' }, 'reset');
}

/** Sets a typed speed; text that is not a whole number from 25 to 200 is refused here. */
export async function speedSet(text: string): Promise<void> {
  if (pl.status !== 'active' || speedLocked()) return;
  const percent = parseSpeed(text);
  if (percent === null) {
    pl.speedError = `Enter a whole number from ${MIN_SPEED} to ${MAX_SPEED} %`;
    log(`player speed refused text=${text}`);
    return;
  }
  await speedCall({ kind: 'speed-set', percent }, 'typed');
}

/** Clears the typed-speed refusal (the next edit). */
export function clearSpeedError(): void {
  pl.speedError = '';
}

/** Sets the speed-training plan of the current loop. */
export async function setTraining(t: TrainingPlan): Promise<void> {
  const index = pl.snap?.current_loop ?? null;
  if (index === null) return;
  await snapshotCall(
    'player_loop',
    (id) => playerLoop(id, { kind: 'training', ...t }),
    () => `player training index=${index} on=${t.on} start=${t.start} end=${t.end} step=${t.step} repeats=${t.repeats}`,
  );
}
