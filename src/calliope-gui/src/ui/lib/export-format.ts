import type { CountInInfo, ExportInfo, ExportKind, ExportSnapshot, TrackExports, TrackRecord } from './ipc';

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
  const left = c.skip_ms > 0 ? `; the ${seconds(c.skip_ms)} s before the first beat are left out` : '';
  return `Count-in: 5 beats at ${c.bpm} BPM, the last one silent before the first beat (adds ${seconds(c.lead_ms)} s${left})`;
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

const pad = (n: number): string => String(n).padStart(2, '0');

/** `YYYY-MM-DD` in local time; '' for an empty or unparseable value. */
export function formatDay(iso: string): string {
  const d = new Date(iso);
  if (iso === '' || Number.isNaN(d.getTime())) return '';
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** `YYYY-MM-DD HH:MM` in local time; '' for an empty or unparseable value. */
export function formatStamp(iso: string): string {
  const day = formatDay(iso);
  if (day === '') return '';
  const d = new Date(iso);
  return `${day} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export interface ExportRow { id: string; label: string; value: string }

/** One read-only row per export kind present: RC-5, MP3, OGG. */
export function exportRows(exports?: TrackExports): ExportRow[] {
  if (!exports) return [];
  const row = (id: string, label: string, what: string, at: string): ExportRow => {
    const stamp = formatStamp(at);
    return { id, label, value: stamp ? `${what} (${stamp})` : what };
  };
  const rows: ExportRow[] = [];
  if (exports.rc5) rows.push(row('track-export-rc5', 'RC-5', `memory ${exports.rc5.memory}`, exports.rc5.at));
  if (exports.mp3) rows.push(row('track-export-mp3', 'MP3', exports.mp3.path, exports.mp3.at));
  if (exports.ogg) rows.push(row('track-export-ogg', 'OGG', exports.ogg.path, exports.ogg.at));
  return rows;
}

/** The "Last:" line under an export button; '' when that kind was never exported. */
export function lastText(kind: ExportKind, last?: TrackExports | null): string {
  const e = last?.[kind];
  if (!e) return '';
  const what = 'memory' in e ? `memory ${e.memory}` : e.path;
  const day = formatDay(e.at);
  return day ? `Last: ${what}, ${day}` : `Last: ${what}`;
}
