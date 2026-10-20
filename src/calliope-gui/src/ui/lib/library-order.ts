import type { AlbumNode, BandNode } from './library-tree';

export interface Patch {
  id: string;
  revision: string;
  band?: string;
  album?: string;
  position?: number | null;
  album_position?: number | null;
}

/** `list` with the item at `from` moved so that it ends at index `to` (both 0-based). */
export function moveItem<T>(list: T[], from: number, to: number): T[] {
  const out = [...list];
  if (from === to || from < 0 || from >= out.length) return out;
  const [item] = out.splice(from, 1);
  out.splice(Math.max(0, Math.min(to, out.length)), 0, item);
  return out;
}

/** Gap index 0..n (0 = before the first) to the final index of an item now at `from`; null = no move. */
export function gapToIndex(from: number, gap: number): number | null {
  const to = gap > from ? gap - 1 : gap;
  return to === from ? null : to;
}

/** Gap under the pointer: upper half of row i gives i, lower half i + 1; null outside [first.top, last.bottom). */
export function gapAt(rects: { top: number; bottom: number }[], y: number): number | null {
  if (rects.length === 0 || y < rects[0].top || y >= rects[rects.length - 1].bottom) return null;
  for (let i = 0; i < rects.length; i++) {
    const r = rects[i];
    if (y >= r.top && y < r.bottom) return y < (r.top + r.bottom) / 2 ? i : i + 1;
  }
  // Between two rows that do not touch: the nearest following gap.
  for (let i = 0; i < rects.length; i++) if (y < rects[i].top) return i;
  return null;
}

/** Patches giving every track of the album, in the new order, positions 1..n; only tracks whose position differs. */
export function trackOrderPatches(album: AlbumNode, from: number, to: number): Patch[] {
  const patches: Patch[] = [];
  moveItem(album.tracks, from, to).forEach((t, i) => {
    if (t.position !== i + 1) patches.push({ id: t.id, revision: t.revision, position: i + 1 });
  });
  return patches;
}

/** The same for the band's real albums ("(no album)" excluded): album_position 1..m on every track of each album. */
export function albumOrderPatches(band: BandNode, from: number, to: number): Patch[] {
  const patches: Patch[] = [];
  moveItem(
    band.albums.filter((a) => !a.isNone),
    from,
    to,
  ).forEach((a, i) => {
    for (const t of a.tracks) patches.push({ id: t.id, revision: t.revision, album_position: i + 1 });
  });
  return patches;
}

/** The first key of `before` after the removed ones that still exists in `after`, else the nearest one before them, else after[0] ?? null. */
export function neighbourKey(before: string[], removed: Set<string>, after: string[]): string | null {
  const present = new Set(after);
  const ok = (k: string): boolean => !removed.has(k) && present.has(k);
  const first = before.findIndex((k) => removed.has(k));
  if (first >= 0) {
    for (let i = first + 1; i < before.length; i++) if (ok(before[i])) return before[i];
    for (let i = first - 1; i >= 0; i--) if (ok(before[i])) return before[i];
  }
  return after[0] ?? null;
}

export const confirmDeleteAlbum = (band: string, album: string, n: number): string =>
  `Delete the album "${album}" by ${band} (${n === 1 ? '1 track' : `${n} tracks`})? The tracks will be moved to the repository's trash folder.`;
