import type { TrackRecord } from './ipc';

/** NFD, strip combining marks, lower-case. */
export function normalize(s: string): string {
  return s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
}

const isWordChar = (c: string): boolean => /[\p{L}\p{N}]/u.test(c);

/**
 * A (normalized) token matches a (normalized) field when it is a substring of it, or a
 * subsequence whose first character sits at a word start.
 */
export function tokenMatches(token: string, field: string): boolean {
  if (token === '') return true;
  if (field.includes(token)) return true;
  const t = [...token];
  const f = [...field];
  for (let start = 0; start < f.length; start++) {
    if (f[start] !== t[0]) continue;
    if (start > 0 && isWordChar(f[start - 1])) continue;
    let ti = 1;
    for (let i = start + 1; i < f.length && ti < t.length; i++) {
      if (f[i] === t[ti]) ti++;
    }
    if (ti === t.length) return true;
  }
  return false;
}

export function queryTokens(query: string): string[] {
  return normalize(query).split(/\s+/).filter((t) => t !== '');
}

/** Every token must match one of band, album or title. An empty query matches everything. */
export function trackMatches(
  track: Pick<TrackRecord, 'band' | 'album' | 'title'>,
  query: string,
): boolean {
  const tokens = queryTokens(query);
  if (tokens.length === 0) return true;
  const fields = [track.band, track.album, track.title].map(normalize);
  return tokens.every((t) => fields.some((f) => tokenMatches(t, f)));
}
