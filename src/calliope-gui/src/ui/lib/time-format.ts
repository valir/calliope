export function formatPosition(ms: number): string {
  if (ms < 0) ms = 0;
  const m = Math.floor(ms / 60000);
  const s = Math.floor(ms / 1000) % 60;
  const t = Math.floor(ms / 100) % 10;
  return `${m}:${String(s).padStart(2, '0')}.${t}`;
}

export function parsePosition(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === '') return null;
  const fullMatch = /^([0-9]+):([0-5][0-9])(?:[.:]([0-9]))?$/u.exec(trimmed);
  if (fullMatch) {
    const m = Number(fullMatch[1]);
    const ss = Number(fullMatch[2]);
    const t = fullMatch[3] !== undefined ? Number(fullMatch[3]) : 0;
    return (m * 60 + ss) * 1000 + t * 100;
  }
  const simpleMatch = /^([0-9]+)(?:\.([0-9]))?$/u.exec(trimmed);
  if (simpleMatch) {
    const s = Number(simpleMatch[1]);
    const t = simpleMatch[2] !== undefined ? Number(simpleMatch[2]) : 0;
    return s * 1000 + t * 100;
  }
  return null;
}
