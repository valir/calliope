import type { TrackRecord } from './ipc';
import type { Patch } from './library-order';
import { buildTree, groupName, type AlbumNode, type BandNode } from './library-tree';

/** `null` clears the field on disk; absent leaves it. */
type PosFields = { position?: number | null; album_position?: number | null };

export type RenamePlan =
  | { kind: 'none' }
  | { kind: 'refused'; reason: 'clear-band' | 'clear-album' | 'both'; message: string }
  | ({ kind: 'join' } & PosFields)
  | {
      kind: 'rename';
      scope: 'band' | 'album';
      from: string;
      to: string;
      count: number;
      merged: boolean;
      message: string;
      main: PosFields;
      group: Patch[];
    };

const plural = (n: number, one: string, many: string): string => (n === 1 ? `1 ${one}` : `${n} ${many}`);
const tracksText = (n: number): string => plural(n, 'track', 'tracks');
const albumsText = (n: number): string => plural(n, 'album', 'albums');

/** Collects, per track, only the fields that differ from the current record. */
class Changes {
  private map = new Map<string, { track: TrackRecord; fields: Omit<Patch, 'id' | 'revision'> }>();

  private entry(t: TrackRecord) {
    let e = this.map.get(t.id);
    if (!e) {
      e = { track: t, fields: {} };
      this.map.set(t.id, e);
    }
    return e;
  }
  band(t: TrackRecord, v: string): void {
    if (t.band !== v) this.entry(t).fields.band = v;
  }
  album(t: TrackRecord, v: string): void {
    if (t.album !== v) this.entry(t).fields.album = v;
  }
  position(t: TrackRecord, v: number): void {
    if ((t.position ?? null) !== v) this.entry(t).fields.position = v;
  }
  albumPosition(t: TrackRecord, v: number): void {
    if ((t.album_position ?? null) !== v) this.entry(t).fields.album_position = v;
  }
  /** `main` = the edited track's position fields, `group` = every other track's patch. */
  split(edited: TrackRecord, order: TrackRecord[]): { main: PosFields; group: Patch[] } {
    const main: PosFields = {};
    const e = this.map.get(edited.id);
    if (e?.fields.position !== undefined) main.position = e.fields.position;
    if (e?.fields.album_position !== undefined) main.album_position = e.fields.album_position;
    const group: Patch[] = [];
    const seen = new Set<string>();
    for (const t of order) {
      if (t.id === edited.id || seen.has(t.id)) continue;
      seen.add(t.id);
      const f = this.map.get(t.id)?.fields;
      if (f && Object.keys(f).length > 0) group.push({ id: t.id, revision: t.revision, ...f });
    }
    return { main, group };
  }
}

const hasPositions = (ts: TrackRecord[]): boolean => ts.some((t) => !!t.position);

/** Renames the tracks of `renamed` into the album `existing` (merge): the renamed ones come after. */
function mergeAlbums(
  ch: Changes,
  existing: AlbumNode,
  renamed: AlbumNode,
  newAlbum: string | null,
  albumPosition: 'own' | number | null,
): void {
  const E = existing.tracks;
  const R = renamed.tracks;
  if (hasPositions([...E, ...R])) {
    E.forEach((t, i) => ch.position(t, i + 1));
    R.forEach((t, i) => ch.position(t, E.length + i + 1));
  }
  const ap = albumPosition === 'own' ? (existing.position ?? renamed.position) : albumPosition;
  if (ap !== null) for (const t of [...E, ...R]) ch.albumPosition(t, ap);
  if (newAlbum !== null) for (const t of R) ch.album(t, newAlbum);
}

const mergeText = (to: string, k: number): string =>
  ` "${to}" already exists (${tracksText(k)}); they will be merged, the renamed ones after the existing ones.`;

function findBand(tree: BandNode[], key: string): BandNode | undefined {
  return tree.find((b) => b.key === `b:${key}`);
}

export function planRename(
  tracks: TrackRecord[],
  edited: TrackRecord,
  band: string,
  album: string,
): RenamePlan {
  const newBand = band.trim();
  const newAlbum = album.trim();
  const oldBand = edited.band.trim();
  const oldAlbum = edited.album.trim();
  const bandChanged = newBand !== oldBand;
  const albumChanged = newAlbum !== oldAlbum;

  if (!bandChanged && !albumChanged) return { kind: 'none' };
  if (bandChanged && albumChanged) {
    return { kind: 'refused', reason: 'both', message: 'Change the band or the album, save, then change the other.' };
  }
  if (bandChanged && newBand === '') {
    return { kind: 'refused', reason: 'clear-band', message: 'The band name cannot be cleared.' };
  }
  if (albumChanged && newAlbum === '') {
    return { kind: 'refused', reason: 'clear-album', message: 'The album name cannot be cleared.' };
  }

  const tree = buildTree(tracks);
  const srcBandKey = groupName(oldBand);
  const srcBand = findBand(tree, srcBandKey);
  const srcAlbumKey = groupName(oldAlbum);
  const srcAlbum = srcBand?.albums.find((a) => a.key === `b:${srcBandKey}|a:${srcAlbumKey}`);

  if (albumChanged) {
    const newKey = groupName(newAlbum);
    const target = srcBand?.albums.find((a) => a.key === `b:${srcBandKey}|a:${newKey}`);
    if (oldAlbum === '') {
      // Rule 4: a "(no album)" track joins an album.
      const join: PosFields = {};
      // It joins at the end, whatever position it kept from before: numbered after the others, or
      // (an album without numbers) its leftover position is cleared.
      if (target && hasPositions(target.tracks)) {
        join.position = Math.max(0, ...target.tracks.map((t) => t.position ?? 0)) + 1;
      } else if (edited.position) {
        join.position = null;
      }
      if (target && target.position !== null) join.album_position = target.position;
      return { kind: 'join', ...join };
    }
    if (!srcAlbum) return { kind: 'none' };
    const ch = new Changes();
    const affected = srcAlbum.tracks;
    let merged = false;
    let message = `Rename the album "${oldAlbum}" to "${newAlbum}" for ${tracksText(affected.length)}?`;
    if (newKey === srcAlbumKey) {
      for (const t of affected) ch.album(t, newAlbum);
    } else if (target) {
      merged = true;
      mergeAlbums(ch, target, srcAlbum, newAlbum, 'own');
      message += mergeText(newAlbum, target.tracks.length);
    } else {
      for (const t of affected) ch.album(t, newAlbum);
    }
    const { main, group } = ch.split(edited, [...(target && merged ? target.tracks : []), ...affected]);
    return {
      kind: 'rename', scope: 'album', from: oldAlbum, to: newAlbum, count: affected.length,
      merged, message, main, group,
    };
  }

  // Band changed (rule 6).
  if (!srcBand) return { kind: 'none' };
  const newKey = groupName(newBand);
  const targetBand = newKey === srcBandKey ? undefined : findBand(tree, newKey);
  const ch = new Changes();
  let affected: TrackRecord[];
  let srcAlbums: AlbumNode[];
  let noAlbumTracks: TrackRecord[];
  if (oldBand === '') {
    if (!srcAlbum || srcAlbum.isNone) return { kind: 'join' };
    if (srcAlbum.tracks.length === 1) return { kind: 'join' };
    affected = srcAlbum.tracks;
    srcAlbums = [srcAlbum];
    noAlbumTracks = [];
  } else {
    affected = srcBand.albums.flatMap((a) => a.tracks);
    srcAlbums = srcBand.albums.filter((a) => !a.isNone);
    noAlbumTracks = srcBand.albums.filter((a) => a.isNone).flatMap((a) => a.tracks);
  }
  for (const t of affected) ch.band(t, newBand);

  let merged = false;
  let message = `Rename the band "${oldBand || '(no band)'}" to "${newBand}" for ${tracksText(affected.length)} in ${albumsText(srcAlbums.length || 1)}?`;
  const order: TrackRecord[] = [];
  if (targetBand) {
    merged = true;
    const targetAlbums = targetBand.albums.filter((a) => !a.isNone);
    const colliding = new Map<string, AlbumNode>(); // source album key -> existing album
    const albumPart = (a: AlbumNode, bandKey: string): string => a.key.slice(`b:${bandKey}|a:`.length);
    for (const s of srcAlbums) {
      const hit = targetAlbums.find((a) => albumPart(a, newKey) === albumPart(s, srcBandKey));
      if (hit) colliding.set(s.key, hit);
    }
    const sequence: { album: AlbumNode; src?: AlbumNode }[] = [
      ...targetAlbums.map((a) => ({ album: a, src: srcAlbums.find((s) => colliding.get(s.key) === a) })),
      ...srcAlbums.filter((s) => !colliding.has(s.key)).map((s) => ({ album: s })),
    ];
    const numbered = sequence.some((x) => x.album.position !== null || (x.src && x.src.position !== null));
    sequence.forEach((x, i) => {
      const ap = numbered ? i + 1 : null;
      if (x.src) {
        mergeAlbums(ch, x.album, x.src, null, ap);
        order.push(...x.album.tracks, ...x.src.tracks);
      } else {
        if (ap !== null) for (const t of x.album.tracks) ch.albumPosition(t, ap);
        order.push(...x.album.tracks);
      }
    });
    message += mergeText(newBand, targetBand.albums.reduce((n, a) => n + a.tracks.length, 0));
  }
  order.push(...noAlbumTracks, ...affected);
  const { main, group } = ch.split(edited, order);
  return {
    kind: 'rename', scope: 'band', from: oldBand || '(no band)', to: newBand, count: affected.length,
    merged, message, main, group,
  };
}
