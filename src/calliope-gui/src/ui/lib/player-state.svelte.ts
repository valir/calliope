import {
  closePlayer,
  EDITOR_SUPERSEDED,
  frontendLog,
  openPlayer,
  playerLeave,
  playerLoop,
  playerMetronome,
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
  type TrackRecord,
} from './ipc';
import { selectTrack } from './library-state.svelte';

export type PlayerStatus = 'inactive' | 'loading' | 'active' | 'error';

const STOPPED: PlayerTransport = {
  playing: false, counting_in: false, resume_pending: false, position_ms: 0,
  loop_playing: false, bpm: 0, tempo_index: null,
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
  /** Refusal of the last tempo/metronome/loop/transport command. */
  actionError: '',
  /** Playback stopped by an audio error. */
  audioError: '',
  /** player.json could not be written. */
  stateError: '',
  treeCollapsed: false,
});

const log = (m: string): void => void frontendLog(m);
const fail = (command: string, err: unknown): void => log(`error player ${command}: ${String(err)}`);
const clean = (err: unknown): string => String(err).replace(/^Error: /, '');

let generation = 0;
let sessionOpen = false;
let key: string | null | undefined = undefined;
const OPEN_ATTEMPTS = 3;

function resetFields(): void {
  pl.message = '';
  pl.progress = 0;
  pl.snap = null;
  pl.transport = { ...STOPPED };
  pl.actionError = '';
  pl.audioError = '';
  pl.stateError = '';
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
      `loops=${s.loops.length} position=${s.transport.position_ms}`,
  );
}

function applySnapshot(s: PlayerSnapshot): void {
  const { transport, ...rest } = s;
  pl.status = 'active';
  pl.trackId = s.id;
  pl.message = '';
  pl.snap = {
    ...rest,
    sources: rest.sources.map((x) => ({ ...x })),
    loops: rest.loops.map((l) => ({ ...l })),
    tempo: { ...rest.tempo, segments: rest.tempo.segments.map((g) => ({ ...g })) },
    metronome: { ...rest.metronome },
  };
  pl.transport = { ...transport };
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
    .then((tr) => { if (gen === generation) pl.transport = tr; })
    .catch((e) => fail('player_leave', e));
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
      pl.transport = t;
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
  }
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
    pl.transport = t;
    pl.audioError = '';
    pl.actionError = '';
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
): Promise<boolean> {
  const id = pl.trackId;
  if (pl.status !== 'active' || !id) return false;
  const gen = generation;
  try {
    const s = await run(id);
    if (gen !== generation) return false;
    applySnapshot(s);
    pl.actionError = '';
    log(logLine(s));
    return true;
  } catch (err) {
    if (gen !== generation) return false;
    pl.actionError = clean(err);
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

export async function setMetronome(on: boolean, countIn: boolean): Promise<void> {
  await snapshotCall(
    'player_metronome',
    (id) => playerMetronome(id, on, countIn),
    (s) => `player metronome on=${s.metronome.on} count_in=${s.metronome.count_in}`,
  );
}

export async function loop(action: LoopAction): Promise<void> {
  const name = action.kind === 'mark' ? `mark-${action.edge}` : action.kind;
  await snapshotCall(
    'player_loop',
    (id) => playerLoop(id, action),
    (s) => {
      const cur = s.current_loop === null ? null : s.loops[s.current_loop];
      return `player loop ${name} index=${s.current_loop ?? 'none'} count=${s.loops.length} complete=${!!cur && cur.end !== null}`;
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
        if (seekPending === null) pl.transport = t;
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
