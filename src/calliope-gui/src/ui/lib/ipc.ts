import { Channel, invoke } from '@tauri-apps/api/core';

export type Theme = 'dark' | 'light';
export interface Settings {
  theme: Theme;
  repository_root: string | null;
  edge_ai_url: string | null;
  keep_original: boolean;
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
export interface SaveTrackRequest { id: string; revision: string; edits: TrackEdits; tablatures: TabEntry[] }
export interface SaveResult { track: TrackRecord; warnings: string[] }

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
export type JobPhase =
  | 'downloading' | 'preparing' | 'ready' | 'uploading' | 'queued' | 'working'
  | 'receiving' | 'saving' | 'saved' | 'failed' | 'cancelled';
export type JobStage = 'download' | 'prepare' | 'server' | 'save';
export interface JobSource { kind: 'url' | 'audio-file' | 'video-file'; label: string }
export interface JobError { stage: JobStage; message: string; http_status: number | null }
/** A stem the server returned but the import dropped as empty (audible < 15 s); `audible_ms` is its audible time. */
export interface DroppedStem { name: string; audible_ms: number }
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
  | { phase: 'cancelled'; back_to: 'source' | 'edit' };

/** Wraps a callback in the Tauri Channel the Rust side streams ImportEvent values into. */
function channel(onEvent: (e: ImportEvent) => void): Channel<ImportEvent> {
  const ch = new Channel<ImportEvent>();
  ch.onmessage = onEvent;
  return ch;
}

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
/** Opens the file dialog in Rust; null when it was cancelled. */
export const importFile = (
  kind: ImportKind,
  onEvent: (e: ImportEvent) => void,
): Promise<JobSnapshot | null> =>
  invoke<JobSnapshot | null>('import_file', { kind, events: channel(onEvent) });
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
}
export type EditorEvent =
  | { kind: 'loading'; id: string; done: number; total: number }
  | ({ kind: 'transport'; id: string } & TransportState)
  | { kind: 'saving'; id: string; progress: number }
  | { kind: 'saved'; id: string; file: string; clipped_samples: number; track: TrackRecord; warnings: string[] }
  | { kind: 'save-failed'; id: string; message: string }
  | { kind: 'save-cancelled'; id: string }
  | { kind: 'audio-error'; id: string; message: string };

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
export const saveBacking = (id: string): Promise<EditorSnapshot> =>
  invoke<EditorSnapshot>('save_backing', { id });
export const cancelBackingSave = (id: string): Promise<void> =>
  invoke<void>('cancel_backing_save', { id });
