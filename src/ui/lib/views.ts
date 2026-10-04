export type ViewId = 'library' | 'import' | 'track' | 'player' | 'playlists' | 'settings';

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
  { id: 'track', label: 'Track', digit: 3, summary: 'One track: stems, assembly, BPM and sections, tablature, MIDI cues.', features: ['gui-backing-track-assembly', 'gui-tablatures', 'gui-manipulate-backing-track'] },
  { id: 'playlists', label: 'Playlists', digit: 4, summary: 'Backing-track playlists.', features: [] },
  { id: 'player', label: 'Player', digit: 5, summary: 'Playback with the tablature view.', features: ['gui-play-backing-track'] },
  { id: 'settings', label: 'Settings', digit: 6, summary: 'MIDI interface, audio output, edge-AI server and theme.', features: ['gui-play-backing-track', 'gui-stem-extracting'] },
];

export interface TrackTab { id: string; label: string; feature: string }
export const TRACK_TABS: readonly TrackTab[] = [
  { id: 'stems', label: 'Stems', feature: 'gui-backing-track-assembly' },
  { id: 'assembly', label: 'Assembly', feature: 'gui-backing-track-assembly' },
  { id: 'tempo', label: 'BPM & sections', feature: 'gui-manipulate-backing-track' },
  { id: 'tablature', label: 'Tablature', feature: 'gui-tablatures' },
  { id: 'cues', label: 'MIDI cues', feature: 'gui-manipulate-backing-track' },
];

export interface SettingsSection { id: string; title: string; text: string; feature: string }
export const SETTINGS_PLACEHOLDERS: readonly SettingsSection[] = [
  { id: 'midi', title: 'MIDI interface', text: 'Choose the USB-MIDI interface that sends clock and patch changes.', feature: 'gui-play-backing-track' },
  { id: 'audio', title: 'Audio output', text: 'Choose where the audio goes.', feature: 'gui-play-backing-track' },
  { id: 'edge-ai', title: 'Edge-AI server', text: 'The ollama server on the LAN used for stem extraction.', feature: 'gui-stem-extracting' },
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
  const m = /^(?:Digit|Numpad)([1-6])$/.exec(e.code);
  return m ? VIEWS[Number(m[1]) - 1].id : null;
}

export function isNavToggleKey(e: KeyLike): boolean {
  return e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && e.code === 'KeyB';
}
