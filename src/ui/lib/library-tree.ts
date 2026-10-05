import type { TrackRecord } from './ipc';
import { normalize, trackMatches, queryTokens } from './fuzzy';

export interface AlbumNode { key: string; label: string; isNone: boolean; tracks: TrackRecord[] }
export interface BandNode { key: string; label: string; isNone: boolean; albums: AlbumNode[] }

export const NO_BAND = '(no band)';
export const NO_ALBUM = '(no album)';

const collator = new Intl.Collator(undefined, { sensitivity: 'base', numeric: true });

function compareText(a: string, b: string): number {
  return collator.compare(a, b) || (a < b ? -1 : a > b ? 1 : 0);
}

function compareTracks(a: TrackRecord, b: TrackRecord): number {
  return compareText(a.title, b.title) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

const groupName = (s: string): string => normalize(s.trim());

function bySortedFirst<T>(items: T[], name: (t: T) => string): string {
  return [...items].map(name).sort(compareText)[0];
}

/** Groups by case-insensitive trimmed name; the label is the first spelling in sort order. */
function group<T>(items: T[], pick: (t: T) => string): Map<string, T[]> {
  const map = new Map<string, T[]>();
  for (const it of items) {
    const k = groupName(pick(it));
    const list = map.get(k);
    if (list) list.push(it);
    else map.set(k, [it]);
  }
  return map;
}

function sortGroups<G extends { label: string; isNone: boolean }>(groups: G[]): G[] {
  return groups.sort((a, b) =>
    a.isNone !== b.isNone ? (a.isNone ? 1 : -1) : compareText(a.label, b.label),
  );
}

export function buildTree(tracks: TrackRecord[]): BandNode[] {
  const bands: BandNode[] = [];
  for (const [bandKey, bandTracks] of group(tracks, (t) => t.band)) {
    const isNone = bandKey === '';
    const albums: AlbumNode[] = [];
    for (const [albumKey, albumTracks] of group(bandTracks, (t) => t.album)) {
      const albumNone = albumKey === '';
      albums.push({
        key: `b:${bandKey}|a:${albumKey}`,
        label: albumNone ? NO_ALBUM : bySortedFirst(albumTracks, (t) => t.album.trim()),
        isNone: albumNone,
        tracks: [...albumTracks].sort(compareTracks),
      });
    }
    bands.push({
      key: `b:${bandKey}`,
      label: isNone ? NO_BAND : bySortedFirst(bandTracks, (t) => t.band.trim()),
      isNone,
      albums: sortGroups(albums),
    });
  }
  return sortGroups(bands);
}

/** Keys of every band and album node. */
export function allGroupKeys(tree: BandNode[]): string[] {
  return tree.flatMap((b) => [b.key, ...b.albums.map((a) => a.key)]);
}

/**
 * Keeps matching tracks (and the branches above them). `expandKeys` lists every kept group
 * while the query is non-empty; the full tree and no keys otherwise.
 */
export function filterTree(
  tree: BandNode[],
  query: string,
): { tree: BandNode[]; expandKeys: string[] } {
  if (queryTokens(query).length === 0) return { tree, expandKeys: [] };
  const out: BandNode[] = [];
  for (const band of tree) {
    const albums: AlbumNode[] = [];
    for (const album of band.albums) {
      const tracks = album.tracks.filter((t) => trackMatches(t, query));
      if (tracks.length > 0) albums.push({ ...album, tracks });
    }
    if (albums.length > 0) out.push({ ...band, albums });
  }
  return { tree: out, expandKeys: allGroupKeys(out) };
}
