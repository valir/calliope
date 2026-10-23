export type ViewId = 'library' | 'import' | 'editor' | 'player' | 'playlists' | 'export' | 'settings';

export interface ViewInfo {
  id: ViewId;
  label: string;
  digit: number;
  summary: string;
  features: string[];
}

export const VIEWS: readonly ViewInfo[] = [
  { id: 'library', label: 'Library', digit: 1, summary: 'The backing-track repository.', features: ['gui-tracks-repository'] },
  { id: 'import', label: 'Import', digit: 2, summary: 'Import a music track for stem extraction, or an existing backing track.', features: ['gui-stem-extracting', 'gui-existing-track-import'] },
  { id: 'editor', label: 'Editor', digit: 3, summary: 'One track: stems and backing mix, BPM and sections, tablature, MIDI cues.', features: ['gui-backing-track-editor', 'gui-tablatures', 'gui-manipulate-backing-track'] },
  { id: 'playlists', label: 'Playlists', digit: 4, summary: 'Backing-track playlists.', features: [] },
  { id: 'player', label: 'Player', digit: 5, summary: 'Playback with the tablature view.', features: ['gui-play-backing-track'] },
  { id: 'export', label: 'Export', digit: 6, summary: "Copy a track's backing to the BOSS RC-5, or save it as MP3 or OGG.", features: ['gui-export'] },
  { id: 'settings', label: 'Settings', digit: 7, summary: 'MIDI interface, audio output, edge-AI server and theme.', features: ['gui-audio-midi-settings', 'gui-stem-extracting'] },
];

export interface TrackTab { id: string; label: string; feature: string }
export const TRACK_TABS: readonly TrackTab[] = [
  { id: 'stems', label: 'Stems', feature: 'gui-backing-track-editor' },
  { id: 'tablature', label: 'Tablature', feature: 'gui-tablatures' },
  { id: 'cues', label: 'MIDI cues', feature: 'gui-manipulate-backing-track' },
];

export interface SettingsSection { id: string; title: string; text: string; feature: string }
export const SETTINGS_PLACEHOLDERS: readonly SettingsSection[] = [
  { id: 'midi', title: 'MIDI interface', text: 'Choose the USB-MIDI interface that sends clock and patch changes.', feature: 'gui-audio-midi-settings' },
  { id: 'audio', title: 'Audio output', text: 'Choose where the audio goes.', feature: 'gui-audio-midi-settings' },
];

export function featureText(features: readonly string[]): string {
  return features.length === 0
    ? 'Planned; there is no feature spec yet.'
    : `This view will be filled by: ${features.join(', ')}.`;
}

export function shortcutLabel(view: ViewInfo): string {
  return `Alt+${view.digit}`;
}

export interface KeyLike {
  code: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}

export function viewForKey(e: KeyLike): ViewId | null {
  if (!e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return null;
  const m = /^(?:Digit|Numpad)([1-7])$/.exec(e.code);
  return m ? VIEWS[Number(m[1]) - 1].id : null;
}

export function isNavToggleKey(e: KeyLike): boolean {
  return e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && e.code === 'KeyB';
}
