import { Channel, invoke } from '@tauri-apps/api/core';

export type Theme = 'dark' | 'light';
export interface Settings {
  theme: Theme;
  repository_root: string | null;
  edge_ai_url: string | null;
  keep_original: boolean;
  last_import_dir?: string | null;
  rc5_dir?: string | null;
  export_dir?: string | null;
  rc5_memory?: number | null;
}

export const appVersion = (): Promise<string> => invoke<string>('app_version');
export const getSettings = (): Promise<Settings> => invoke<Settings>('get_settings');
export const setTheme = (theme: Theme): Promise<Settings> => invoke<Settings>('set_theme', { theme });
export const frontendLog = (message: string): Promise<void> =>
  invoke<void>('frontend_log', { message }).catch(() => undefined);

// Track repository: mirrors src/track_meta.rs, src/repository.rs and src/ipc.rs.
export type TrackType = 'backing' | 'stem';
export interface StemEntry { name: string; file: string }
export interface BackingVariant {
  id: string;
  name: string;
  /** Always `backings/<file name>`. */
  file: string;
  created: string;
  modified: string;
  sample_rate: number;
  bits: number;
  mix: unknown;
}
export interface Rc5Export { memory: number; at: string }
export interface FileExport { path: string; at: string }
export interface TrackExports { rc5: Rc5Export | null; mp3: FileExport | null; ogg: FileExport | null }
export interface TrackRecord {
  id: string;
  type: TrackType;
  band: string;
  album: string;
  title: string;
  composers: string[];
  year: number | null;
  source_url: string | null;
  copyright: string | null;
  /** null for stem tracks. */
  audio: string | null;
  original: string | null;
  stems: StemEntry[];
  stem_model: string | null;
  tablatures: string[];
  backings: BackingVariant[];
  imported: string;
  modified: string;
  revision: string;
  missing: string[];
  /** Place in the album (1-based). Rust always sends it; optional here so test literals stay valid. */
  position?: number | null;
  /** The album's place among the band's albums (1-based). */
  album_position?: number | null;
  /** The last export to each destination. Rust always sends it; optional here so test literals stay valid. */
  exports?: TrackExports;
}
export interface LibraryProblem { dir: string; message: string }
export interface Library { root: string; tracks: TrackRecord[]; problems: LibraryProblem[] }
export type RepoStatus = 'ok' | 'empty' | 'other' | 'missing' | 'newer';
export interface RepoInfo { root: string; is_default: boolean; status: RepoStatus }
export interface PickedRoot { token: string; root: string; status: RepoStatus }
export interface Picked { token: string; name: string }
export type PickPurpose = 'add' | 'update';
export interface TrackEdits {
  band: string;
  album: string;
  title: string;
  composers: string[];
  year: number | null;
  source_url: string | null;
  copyright: string | null;
}
export type TabEntry =
  | { kind: 'keep'; name: string }
  | { kind: 'add'; token: string }
  | { kind: 'replace'; name: string; token: string };
/** One track of a batch change; absent fields stay as they are. */
export interface TrackPatch {
  id: string;
  revision: string;
  band?: string;
  album?: string;
  /** Absent = unchanged, `null` = clear. */
  position?: number | null;
  album_position?: number | null;
}
export interface TrackRef { id: string; revision: string }
export interface BatchFailure { id: string; title: string; message: string }
/** `tracks` holds the re-read record of every patched track whose write did not fail. */
export interface UpdateResult { tracks: TrackRecord[]; failed: BatchFailure[] }
export interface DeleteResult { deleted: string[]; failed: BatchFailure[] }
export interface SaveTrackRequest {
  id: string;
  revision: string;
  edits: TrackEdits;
  tablatures: TabEntry[];
  /** Absent = unchanged, `null` = clear. */
  position?: number | null;
  album_position?: number | null;
  /** Other tracks changed together with this one (band / album renames). */
  group?: TrackPatch[];
}
export interface SaveResult {
  track: TrackRecord;
  warnings: string[];
  updated?: TrackRecord[];
  failed?: BatchFailure[];
}

export const getRepository = (): Promise<RepoInfo> => invoke<RepoInfo>('get_repository');
export const chooseRepositoryRoot = (): Promise<PickedRoot | null> =>
  invoke<PickedRoot | null>('choose_repository_root');
export const setRepositoryRoot = (token: string): Promise<RepoInfo> =>
  invoke<RepoInfo>('set_repository_root', { token });
export const resetRepositoryRoot = (): Promise<RepoInfo> => invoke<RepoInfo>('reset_repository_root');
export const listTracks = (): Promise<Library> => invoke<Library>('list_tracks');
export const pickTablature = (purpose: PickPurpose): Promise<Picked | null> =>
  invoke<Picked | null>('pick_tablature', { purpose });
export const saveTrack = (request: SaveTrackRequest): Promise<SaveResult> =>
  invoke<SaveResult>('save_track', { request });
export const deleteTrack = (id: string, revision: string): Promise<void> =>
  invoke<void>('delete_track', { id, revision });
export const updateTracks = (patches: TrackPatch[]): Promise<UpdateResult> =>
  invoke<UpdateResult>('update_tracks', { patches });
export const deleteTracks = (items: TrackRef[]): Promise<DeleteResult> =>
  invoke<DeleteResult>('delete_tracks', { items });
export const exportTrack = (id: string): Promise<string | null> =>
  invoke<string | null>('export_track', { id });
export const exportTablature = (id: string, name: string): Promise<string | null> =>
  invoke<string | null>('export_tablature', { id, name });

// Stem extraction import: mirrors src/import_job.rs and src/ipc.rs.
export type EdgeAiState = 'not-configured' | 'connected' | 'unreachable' | 'incompatible';
export interface EdgeAiStatus { state: EdgeAiState; message: string; models: string[] }
export interface ToolInfo { found: boolean; version: string | null; ok: boolean; message: string }
export interface ToolsInfo { yt_dlp: ToolInfo; ffmpeg: ToolInfo; ffprobe: ToolInfo }
export type UrlPrepStatus = 'invalid' | 'ready' | 'partial' | 'busy';
export interface UrlPrep { status: UrlPrepStatus; message: string; partial_bytes: number }
export type ImportKind = 'audio' | 'video';
export type PickKind = ImportKind | 'directory';
export type JobPhase =
  | 'downloading' | 'preparing' | 'ready' | 'uploading' | 'queued' | 'working'
  | 'receiving' | 'saving' | 'saved' | 'failed' | 'cancelled' | 'finished';
export type JobStage = 'download' | 'prepare' | 'server' | 'save';
export interface JobSource { kind: 'url' | 'audio-file' | 'video-file' | 'directory'; label: string }
export interface JobError { stage: JobStage; message: string; http_status: number | null }
/** A stem the server returned but the import dropped as empty (audible < 15 s); `audible_ms` is its audible time. */
export interface DroppedStem { name: string; audible_ms: number }
export type BatchItem = { index: number; name: string } & (
  | { result: 'saved'; track: TrackRecord; dropped: DroppedStem[] }
  | { result: 'failed'; stage: JobStage; message: string });
export interface BatchSnapshot { dir: string; files: string[]; current: number | null; items: BatchItem[] }
export interface JobSnapshot {
  job: string;
  source: JobSource;
  phase: JobPhase;
  downloaded: number;
  total: number | null;
  sent: number;
  stems_done: number;
  stems_total: number;
  progress: number | null;
  duration_s: number | null;
  metadata: TrackEdits | null;
  error: JobError | null;
  track: TrackRecord | null;
  dropped: DroppedStem[];
  batch?: BatchSnapshot | null;
}
export type ImportEvent =
  | { phase: 'downloading'; downloaded: number; total: number | null }
  | { phase: 'preparing' }
  | { phase: 'ready'; metadata: TrackEdits; duration_s: number | null }
  | { phase: 'uploading'; sent: number; total: number }
  | { phase: 'queued' }
  | { phase: 'working'; progress: number | null }
  | { phase: 'receiving'; done: number; total: number }
  | { phase: 'saving' }
  | { phase: 'saved'; track: TrackRecord; dropped: DroppedStem[] }
  | { phase: 'failed'; stage: JobStage; message: string; http_status: number | null }
  | { phase: 'cancelled'; back_to: 'source' | 'edit' }
  | { phase: 'batch-file'; index: number; total: number; name: string }
  | { phase: 'batch-item'; item: BatchItem }
  | { phase: 'finished'; batch: BatchSnapshot };

/** Wraps a callback in the Tauri Channel the Rust side streams ImportEvent values into. */
function channel(onEvent: (e: ImportEvent) => void): Channel<ImportEvent> {
  const ch = new Channel<ImportEvent>();
  ch.onmessage = onEvent;
  return ch;
}

export interface PickedFile { path: string; name: string }
export const pickImportFile = (kind: PickKind, typed: string): Promise<PickedFile | null> =>
  invoke<PickedFile | null>('pick_import_file', { kind, typed });
export const importPath = (
  kind: ImportKind, path: string, onEvent: (e: ImportEvent) => void,
): Promise<JobSnapshot> =>
  invoke<JobSnapshot>('import_path', { kind, path, events: channel(onEvent) });
export const importDirectory = (path: string, onEvent: (e: ImportEvent) => void): Promise<JobSnapshot> =>
  invoke<JobSnapshot>('import_directory', { path, events: channel(onEvent) });

export const setEdgeAiUrl = (url: string | null): Promise<Settings> =>
  invoke<Settings>('set_edge_ai_url', { url });
export const setKeepOriginal = (keep: boolean): Promise<Settings> =>
  invoke<Settings>('set_keep_original', { keep });
export const checkEdgeAi = (): Promise<EdgeAiStatus> => invoke<EdgeAiStatus>('check_edge_ai');
export const checkTools = (): Promise<ToolsInfo> => invoke<ToolsInfo>('check_tools');
export const prepareUrlImport = (url: string): Promise<UrlPrep> =>
  invoke<UrlPrep>('prepare_url_import', { url });
export const startUrlImport = (
  url: string,
  resume: boolean,
  onEvent: (e: ImportEvent) => void,
): Promise<JobSnapshot> => invoke<JobSnapshot>('start_url_import', { url, resume, events: channel(onEvent) });
export const startStemExtraction = (job: string, edits: TrackEdits): Promise<JobSnapshot> =>
  invoke<JobSnapshot>('start_stem_extraction', { job, edits });
export const cancelImport = (job: string): Promise<JobSnapshot> =>
  invoke<JobSnapshot>('cancel_import', { job });
export const discardImport = (job: string): Promise<void> =>
  invoke<void>('discard_import', { job });
export const getImportJob = (): Promise<JobSnapshot | null> =>
  invoke<JobSnapshot | null>('get_import_job');
export const watchImport = (onEvent: (e: ImportEvent) => void): Promise<JobSnapshot | null> =>
  invoke<JobSnapshot | null>('watch_import', { events: channel(onEvent) });

// Backing-track editor: mirrors src/editor.rs and src/ipc.rs.
export interface LaneState { name: string; /** null = Off */ gain_db: number | null; unmuted: boolean }
export interface TransportState {
  /** shows_playing */
  playing: boolean;
  position_ms: number;
  resume_pending: boolean;
  solo: string | null;
  clipping: boolean;
}
export type JobKind = 'save' | 'normalize';
export type LoudnessOutcome = 'normalized' | 'limited' | 'at-target' | 'at-ceiling' | 'too-quiet';
export interface LoudnessResult {
  outcome: LoudnessOutcome;
  lufs: number | null;
  true_peak: number | null;
  gain_db: number;
  target: number;
  ceiling: number;
}
export interface EditorSnapshot {
  id: string;
  title: string;
  stems: LaneState[];
  duration_ms: number;
  sample_rate: number;
  transport: TransportState;
  variant: { id: string; name: string; file: string; exists: boolean };
  /** 0..1 while a save runs */
  saving: number | null;
  /** the running save or normalize job, if any */
  job?: JobKind | null;
  /** peaks of the checked-stem mix; empty until computed */
  peaks: number[];
  tempo: TempoSnap;
  /** the running beat detection, if any */
  detect: DetectState | null;
}
export interface TempoSegment {
  start_ms: number;
  bpm: number;
  /** set on changes found by beat detection */
  detected_ms?: number | null;
  /** first section only: set when the first beat was found or placed; `start_ms` is the first beat then */
  found_ms?: number | null;
  /** set on changes added or moved in the Player: the section starts on a beat */
  on_beat?: boolean;
  /** the section's name; missing = no name */
  name?: string;
  /** the section's metronome; missing = on */
  metronome?: boolean;
  /** a count-in before this section while playing through its start; missing = off */
  count_in?: boolean;
}
export interface TempoSnap {
  beats_per_bar: number;
  segments: TempoSegment[];
  multi: boolean;
}
export interface DetectState {
  phase: 'checking' | 'mixing' | 'uploading' | 'waiting' | 'detecting' | 'applying';
  progress: number | null;
}
/** `steps` is the absolute offset from `detected_ms`, -256..256; for index 0 from `found_ms` (else the start), clamped to the track. */
export type TempoEdit = { kind: 'move'; index: number; steps: number } | { kind: 'reload' };
export type EditorEvent =
  | { kind: 'loading'; id: string; done: number; total: number }
  | ({ kind: 'transport'; id: string } & TransportState)
  | { kind: 'saving'; id: string; progress: number; job?: JobKind }
  | { kind: 'saved'; id: string; file: string; clipped_samples: number; loudness?: LoudnessResult; track: TrackRecord; warnings: string[] }
  | { kind: 'normalized'; id: string; file: string; loudness: LoudnessResult; changed: boolean; track: TrackRecord | null; warnings: string[] }
  | { kind: 'save-failed'; id: string; message: string; job?: JobKind }
  | { kind: 'save-cancelled'; id: string; job?: JobKind }
  | { kind: 'audio-error'; id: string; message: string }
  | { kind: 'peaks'; id: string; peaks: number[] }
  | ({ kind: 'detect'; id: string } & DetectState)
  | { kind: 'detected'; id: string; tempo: TempoSnap; beats: number; replaced: boolean }
  | { kind: 'detect-failed'; id: string; message: string }
  | { kind: 'detect-cancelled'; id: string };

function editorChannel(onEvent: (e: EditorEvent) => void): Channel<EditorEvent> {
  const ch = new Channel<EditorEvent>();
  ch.onmessage = onEvent;
  return ch;
}

/** A superseded load fails with this message; the UI ignores it. */
export const EDITOR_SUPERSEDED = 'superseded';

export const openEditor = (id: string, onEvent: (e: EditorEvent) => void): Promise<EditorSnapshot> =>
  invoke<EditorSnapshot>('open_editor', { id, events: editorChannel(onEvent) });
export const closeEditor = (): Promise<void> => invoke<void>('close_editor');
export const getEditor = (): Promise<EditorSnapshot | null> => invoke<EditorSnapshot | null>('get_editor');
export const watchEditor = (onEvent: (e: EditorEvent) => void): Promise<EditorSnapshot | null> =>
  invoke<EditorSnapshot | null>('watch_editor', { events: editorChannel(onEvent) });
export const editorPlay = (id: string): Promise<TransportState> =>
  invoke<TransportState>('editor_play', { id });
export const editorLanePlay = (id: string, name: string): Promise<TransportState> =>
  invoke<TransportState>('editor_lane_play', { id, name });
export const editorEndSolo = (id: string): Promise<TransportState> =>
  invoke<TransportState>('editor_end_solo', { id });
export const editorPause = (id: string): Promise<TransportState> =>
  invoke<TransportState>('editor_pause', { id });
export const editorStop = (id: string): Promise<TransportState> =>
  invoke<TransportState>('editor_stop', { id });
/** `position` in milliseconds. */
export const editorSeek = (id: string, position: number): Promise<TransportState> =>
  invoke<TransportState>('editor_seek', { id, position });
/** `delta` in milliseconds (negative = back). */
export const editorNudge = (id: string, delta: number): Promise<TransportState> =>
  invoke<TransportState>('editor_nudge', { id, delta });
/** `gain` in dB, null = Off. */
export const editorSetStem = (
  id: string,
  name: string,
  gain: number | null,
  unmuted: boolean,
): Promise<LaneState> => invoke<LaneState>('editor_set_stem', { id, name, gain, unmuted });
export const editorDetect = (id: string): Promise<EditorSnapshot> =>
  invoke<EditorSnapshot>('editor_detect', { id });
export const editorCancelDetect = (id: string): Promise<void> => invoke<void>('editor_cancel_detect', { id });
export const editorTempo = (id: string, action: TempoEdit): Promise<EditorSnapshot> =>
  invoke<EditorSnapshot>('editor_tempo', { id, action });
export const saveBacking = (id: string): Promise<EditorSnapshot> =>
  invoke<EditorSnapshot>('save_backing', { id });
export const normalizeBacking = (id: string): Promise<EditorSnapshot> =>
  invoke<EditorSnapshot>('normalize_backing', { id });
export const cancelBackingSave = (id: string): Promise<void> =>
  invoke<void>('cancel_backing_save', { id });

// Player: mirrors src/player.rs and src/player_ipc.rs.
export type Boundary = { tick: number } | { ms: number };
export interface PlayerLoop {
  id: string;
  start: Boundary;
  end: Boundary | null;
  start_ms: number;
  end_ms: number | null;
  repeat: boolean;
  count_in: boolean;
  /** speed-training plan of this loop (defaults 70/100/5/2, off, when the file has none) */
  training: TrainingPlan;
}
/** Speed training: percentages start..end in steps of `step`, `repeats` passes at each. */
export interface TrainingPlan {
  on: boolean;
  start: number;
  end: number;
  step: number;
  repeats: number;
}
/** Where a running training run is: loop index, heard speed, pass number, and passes at this speed (null at the end speed). */
export interface TrainingProgress {
  loop: number;
  speed_pct: number;
  repeat: number;
  of: number | null;
}
export interface PlayerTransport {
  /** playing, counting in or resume pending */
  playing: boolean;
  counting_in: boolean;
  resume_pending: boolean;
  position_ms: number;
  loop_playing: boolean;
  /** song BPM at the position (without speed), 0 = none */
  bpm: number;
  /** BPM actually heard: bpm * speed, 0 = none */
  played_bpm: number;
  /** heard speed in percent; lags one engine callback after a speed action */
  speed_pct: number;
  tempo_index: number | null;
  /** running speed-training pass, null when none */
  training: TrainingProgress | null;
}
export interface PlayerSnapshot {
  id: string;
  title: string;
  sources: { key: string; label: string }[];
  source: string;
  duration_ms: number;
  sample_rate: number;
  /** 2048 values 0..1 */
  peaks: number[];
  mode: 'bpm' | 'time';
  tempo: TempoSnap;
  /** track speed in percent */
  speed_pct: number;
  loops: PlayerLoop[];
  /** index */
  current_loop: number | null;
  loop_message: string | null;
  transport: PlayerTransport;
  /** settings file note */
  note: string | null;
  /** the track's Player settings were written by a newer Calliope; they can't be changed */
  read_only: boolean;
  /** the running BPM detection, null when none */
  bpm_detect: BpmDetectView | null;
}
export type DetectPhase = 'checking' | 'uploading' | 'waiting' | 'detecting';
export interface BpmDetectView {
  serial: number;
  index: number;
  at: number;
  from_ms: number;
  to_ms: number;
  source: string;
  phase: DetectPhase;
}
export interface BpmResult {
  serial: number;
  index: number;
  outcome: 'applied' | 'failed' | 'dropped';
  old_bpm: number;
  new_bpm: number | null;
  from_ms: number;
  to_ms: number;
  hint: 'double' | 'half' | null;
  message: string | null;
}
export type PlayerEvent =
  | { kind: 'loading'; id: string; progress: number }
  | ({ kind: 'transport'; id: string } & PlayerTransport)
  | { kind: 'audio-error'; id: string; message: string }
  | { kind: 'state-error'; id: string; message: string }
  | { kind: 'closed'; id: string }
  | { kind: 'bpm-detect'; id: string; detect: BpmDetectView }
  | { kind: 'bpm-detected'; id: string; result: BpmResult; snapshot: PlayerSnapshot | null };
export type TempoAction =
  | { kind: 'set-bpm'; bpm: number }
  | { kind: 'speed-step'; steps: number }
  | { kind: 'speed-reset' }
  | { kind: 'find-start' }
  | { kind: 'speed-set'; percent: number }
  | { kind: 'section-bpm'; index: number; at: number; bpm: number }
  | { kind: 'scale-section'; index: number; at: number; factor: 'double' | 'halve' }
  | { kind: 'add-change'; position: number; bpm: number }
  | { kind: 'remove-change'; index: number; at: number }
  | { kind: 'move-change'; index: number; at: number; steps: number }
  | { kind: 'section-flags'; index: number; at: number; metronome: boolean; count_in: boolean }
  | { kind: 'section-name'; index: number; at: number; name: string }
  | { kind: 'jump-section'; index: number; at: number; edge: 'start' | 'end' };
export type LoopAction =
  | { kind: 'add' }
  | { kind: 'remove' }
  | { kind: 'select'; delta: -1 | 1 }
  | { kind: 'mark'; edge: 'start' | 'end' }
  | { kind: 'set'; edge: 'start' | 'end'; value: Boundary }
  | { kind: 'flags'; repeat: boolean; count_in: boolean }
  | { kind: 'play' }
  | { kind: 'stop' }
  | { kind: 'training'; on: boolean; start: number; end: number; step: number; repeats: number };

function playerChannel(onEvent: (e: PlayerEvent) => void): Channel<PlayerEvent> {
  const ch = new Channel<PlayerEvent>();
  ch.onmessage = onEvent;
  return ch;
}

export const openPlayer = (id: string, onEvent: (e: PlayerEvent) => void): Promise<PlayerSnapshot> =>
  invoke<PlayerSnapshot>('open_player', { id, events: playerChannel(onEvent) });
export const closePlayer = (): Promise<void> => invoke<void>('close_player');
export const getPlayer = (): Promise<PlayerSnapshot | null> => invoke<PlayerSnapshot | null>('get_player');
export const watchPlayer = (onEvent: (e: PlayerEvent) => void): Promise<PlayerSnapshot | null> =>
  invoke<PlayerSnapshot | null>('watch_player', { events: playerChannel(onEvent) });
export const playerSelectSource = (id: string, source: string): Promise<PlayerSnapshot> =>
  invoke<PlayerSnapshot>('player_select_source', { id, source });
export const playerPlay = (id: string): Promise<PlayerTransport> =>
  invoke<PlayerTransport>('player_play', { id });
export const playerPause = (id: string): Promise<PlayerTransport> =>
  invoke<PlayerTransport>('player_pause', { id });
export const playerSeek = (id: string, position: number): Promise<PlayerTransport> =>
  invoke<PlayerTransport>('player_seek', { id, position });
export const playerNudge = (id: string, delta: number): Promise<PlayerTransport> =>
  invoke<PlayerTransport>('player_nudge', { id, delta });
export const playerSkip = (id: string, delta: number): Promise<PlayerTransport> =>
  invoke<PlayerTransport>('player_skip', { id, delta });
export const playerLeave = (id: string): Promise<PlayerTransport> =>
  invoke<PlayerTransport>('player_leave', { id });
export const playerTempo = (id: string, action: TempoAction): Promise<PlayerSnapshot> =>
  invoke<PlayerSnapshot>('player_tempo', { id, action });
export const playerDetectBpm = (id: string, index: number, at: number): Promise<PlayerSnapshot> =>
  invoke<PlayerSnapshot>('player_detect_bpm', { id, index, at });
export const playerCancelDetect = (id: string): Promise<PlayerSnapshot> =>
  invoke<PlayerSnapshot>('player_cancel_detect', { id });
export const playerLoop = (id: string, action: LoopAction): Promise<PlayerSnapshot> =>
  invoke<PlayerSnapshot>('player_loop', { id, action });

// Export: mirrors src/export_job.rs and src/export_ipc.rs.
export type ExportKind = 'rc5' | 'mp3' | 'ogg';
export type ExportTargetReq = { kind: 'rc5'; memory: string } | { kind: 'mp3' } | { kind: 'ogg' };
export interface CountInInfo { bpm: number; first_beat_ms: number; lead_ms: number; skip_ms: number }
export interface ExportInfo {
  id: string;
  available: boolean;
  reason: string | null;
  source: string | null;
  count_in: CountInInfo | null;
  memory: number;
  last: TrackExports;
}
export interface ExportPlan { token: string; path: string; exists: boolean }
export type ExportPhase = 'writing' | 'finishing' | 'done' | 'failed' | 'cancelled';
export interface ExportSnapshot {
  job: string;
  id: string;
  title: string;
  target: ExportKind;
  path: string;
  phase: ExportPhase;
  progress: number;
  message: string | null;
  clipped: number;
  info_error: string | null;
}
export type ExportEvent =
  | { phase: 'writing'; progress: number }
  | { phase: 'finishing' }
  | {
    phase: 'done'; path: string; clipped: number; root: string; records: TrackRecord[];
    info_error: string | null;
  }
  | { phase: 'failed'; message: string }
  | { phase: 'cancelled' };

function exportChannel(onEvent: (e: ExportEvent) => void): Channel<ExportEvent> {
  const ch = new Channel<ExportEvent>();
  ch.onmessage = onEvent;
  return ch;
}

export const exportInfo = (id: string): Promise<ExportInfo> => invoke<ExportInfo>('export_info', { id });
export const prepareExport = (id: string, target: ExportTargetReq): Promise<ExportPlan | null> =>
  invoke<ExportPlan | null>('prepare_export', { id, target });
export const startExport = (
  token: string,
  overwrite: boolean,
  onEvent: (e: ExportEvent) => void,
): Promise<ExportSnapshot> =>
  invoke<ExportSnapshot>('start_export', { token, overwrite, events: exportChannel(onEvent) });
export const cancelExport = (job: string): Promise<ExportSnapshot | null> =>
  invoke<ExportSnapshot | null>('cancel_export', { job });
export const getExport = (): Promise<ExportSnapshot | null> => invoke<ExportSnapshot | null>('get_export');
export const watchExport = (onEvent: (e: ExportEvent) => void): Promise<ExportSnapshot | null> =>
  invoke<ExportSnapshot | null>('watch_export', { events: exportChannel(onEvent) });
