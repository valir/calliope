/** Bytes as megabytes with one decimal ("12.3 MB"). */
export function formatMb(bytes: number): string {
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** Seconds as m:ss (null/unknown = ""). */
export function formatLength(seconds: number | null): string {
  if (seconds === null) return '';
  const s = Math.round(seconds);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
