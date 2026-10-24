import type { CountInInfo, ExportInfo, ExportSnapshot, TrackRecord } from './ipc';

export type ExportTarget =
  | { kind: 'group' }
  | { kind: 'track'; track: TrackRecord }
  | { kind: 'none' };

export const SELECT_TRACK_TEXT = 'Select a track to export it';
export const BUSY_TEXT = 'An export is running';
export const CHECKING_TEXT = 'Checking the track...';

/** A band or album tree key (`b:x`, `b:x|a:y`) is a group; the focused row decides, else the selected track. */
export function exportTarget(treeFocus: string | null, selected: TrackRecord | null): ExportTarget {
  if (treeFocus !== null && treeFocus.startsWith('b:')) return { kind: 'group' };
  return selected ? { kind: 'track', track: selected } : { kind: 'none' };
}

export function percent(progress: number): number {
  return Math.max(0, Math.min(100, Math.round(progress * 100)));
}

export function progressText(snap: ExportSnapshot): string {
  return `Exporting ${snap.title} to ${snap.path} · ${percent(snap.progress)} %`;
}

function seconds(ms: number): string {
  return (ms / 1000).toFixed(1);
}

export function countInText(info: { count_in: CountInInfo | null } | null): string {
  const c = info?.count_in;
  if (!c) return '';
  return `Count-in: 5 beats at ${c.bpm} BPM, the last one silent before the first beat (adds ${seconds(c.lead_ms)} s)`;
}

export function resultText(snap: ExportSnapshot | null): string {
  if (!snap) return '';
  switch (snap.phase) {
    case 'done':
      return `Written to ${snap.path}.` + (snap.target === 'rc5' ? ' Eject the RC-5 before unplugging it.' : '');
    case 'failed':
      return `Export failed: ${snap.message ?? 'unknown error'}`;
    case 'cancelled':
      return 'Export cancelled; nothing was written.';
    default:
      return '';
  }
}

export function confirmText(path: string): string {
  return `Replace the existing file ${path}? Its old content will be lost.`;
}

/** null = the buttons are enabled. */
export function disabledReason(target: ExportTarget, info: ExportInfo | null, busy: boolean): string | null {
  if (busy) return BUSY_TEXT;
  if (target.kind !== 'track') return SELECT_TRACK_TEXT;
  if (!info || info.id !== target.track.id) return CHECKING_TEXT;
  return info.available ? null : (info.reason ?? 'This track cannot be exported');
}
