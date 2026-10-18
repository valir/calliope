import type { TrainingProgress } from './ipc';
import { formatBpm } from './bpm-format';

export const MIN_SPEED = 25;
export const MAX_SPEED = 200;

/** A typed speed: whole number 25..200, optional "%" and spaces; else null. */
export function parseSpeed(text: string): number | null {
  const m = /^\s*(\d{1,3})\s*%?\s*$/.exec(text);
  if (!m) return null;
  const v = Number(m[1]);
  return v >= MIN_SPEED && v <= MAX_SPEED ? v : null;
}

/** "60 BPM" (formatBpm of the value), or "" for 0. */
export function readoutText(playedBpm: number): string {
  return playedBpm === 0 ? '' : `${formatBpm(Math.round(playedBpm * 10) / 10)} BPM`;
}

/** "70 % · 2/4", or "80 % · 3" at the end speed (of === null). */
export function formatTraining(p: TrainingProgress): string {
  return `${p.speed_pct} % · ${p.of === null ? p.repeat : `${p.repeat}/${p.of}`}`;
}

/** A training box value: whole number within [min, max]; else null. */
export function parseTrainingField(text: string, min: number, max: number): number | null {
  if (!/^\s*\d{1,4}\s*$/.test(text)) return null;
  const v = Number(text);
  return v >= min && v <= max ? v : null;
}
