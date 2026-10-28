import type { JobKind, LoudnessResult } from './ipc';

/** One decimal, ASCII minus, never "-0.0". */
function num(v: number): string {
  const t = v.toFixed(1);
  return t === '-0.0' ? '0.0' : t;
}
function signed(v: number): string {
  const t = num(v);
  return t.startsWith('-') ? t : `+${t}`;
}
/** Whole numbers without decimals (the ceiling: `-1`). */
function whole(v: number): string {
  return Number.isInteger(v) ? String(v) : num(v);
}

/** The result sentence of a Save or a Normalize. */
export function loudnessText(r: LoudnessResult, job: JobKind): string {
  const lufs = r.lufs === null ? '' : num(r.lufs);
  const target = num(r.target);
  const ceiling = whole(r.ceiling);
  const gain = signed(r.gain_db);
  const save = job === 'save';
  switch (r.outcome) {
    case 'limited':
      return `${save ? 'Saved at' : 'Normalized to'} ${lufs} LUFS (gain ${gain} dB, limited by the ${ceiling} dBTP peak ceiling).`;
    case 'too-quiet':
      return save
        ? 'Saved. Too quiet or too short to normalize.'
        : 'Too quiet or too short to normalize; the file was not changed.';
    case 'at-target':
      if (!save) return `Already at the target (${lufs || target} LUFS); the file was not changed.`;
      break;
    case 'at-ceiling':
      if (!save) return `Already at the ${ceiling} dBTP peak ceiling (${lufs} LUFS); the file was not changed.`;
      break;
  }
  return `${save ? 'Saved, normalized to' : 'Normalized to'} ${lufs || target} LUFS (gain ${gain} dB).`;
}
