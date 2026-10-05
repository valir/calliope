import { invoke } from '@tauri-apps/api/core';

export type Theme = 'dark' | 'light';
export interface Settings { theme: Theme; repository_root: string | null }

export const appVersion = (): Promise<string> => invoke<string>('app_version');
export const getSettings = (): Promise<Settings> => invoke<Settings>('get_settings');
export const setTheme = (theme: Theme): Promise<Settings> => invoke<Settings>('set_theme', { theme });
export const frontendLog = (message: string): Promise<void> =>
  invoke<void>('frontend_log', { message }).catch(() => undefined);

// Track repository: mirrors src/track_meta.rs, src/repository.rs and src/ipc.rs.
export interface TrackRecord {
  id: string;
  band: string;
  album: string;
  title: string;
  composers: string[];
  year: number | null;
  source_url: string | null;
  copyright: string | null;
  audio: string;
  tablatures: string[];
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
