export function formatGain(db: number | null): string {
  if (db === null) return 'Off';
  if (db <= -60) return 'Off';
  if (db === 0) return '0.0 dB';
  if (db > 0) return `+${db.toFixed(1)} dB`;
  return `${db.toFixed(1)} dB`;
}

export function sliderToGain(v: number): number | null {
  if (v <= -60) return null;
  return v;
}

export function gainToSlider(db: number | null): number {
  if (db === null) return -60;
  return Math.max(-60, Math.min(12, db));
}
