import { describe, it, expect } from 'vitest';
import { planRename } from './library-rename';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { TrackRecord } from './ipc';

const mk = (id: string, band: string, album: string, position: number | null = null,
  album_position: number | null = null): TrackRecord => ({
  ...FIXTURE_TRACKS[0], id, band, album, title: id, position, album_position, revision: `r-${id}`,
});
const album = (band: string, name: string, n: number, ap: number | null = null, prefix = name): TrackRecord[] =>
  Array.from({ length: n }, (_, i) => mk(`${prefix}${i + 1}`, band, name, i + 1, ap));
const rename = (p: ReturnType<typeof planRename>) => {
  if (p.kind !== 'rename') throw new Error(`expected rename, got ${p.kind}`);
  return p;
};
const plan = (ts: TrackRecord[], id: string, band: string, albumName: string) =>
  planRename(ts, ts.find((t) => t.id === id)!, band, albumName);

describe('planRename: album', () => {
  const ts = [...album('Metalica', 'Masterof Puppets', 8, 1, 'm'), ...album('Metalica', 'Ride the Lightning', 2, 2, 'r')];
  it('renames the album for every track, keeping the positions', () => {
    const p = rename(plan(ts, 'm3', 'Metalica', 'Master of Puppets'));
    expect(p.scope).toBe('album');
    expect(p.count).toBe(8);
    expect(p.merged).toBe(false);
    expect(p.group).toHaveLength(7);
    expect(p.group.every((g) => g.album === 'Master of Puppets' && g.position === undefined)).toBe(true);
    expect(p.group[0]).toEqual({ id: 'm1', revision: 'r-m1', album: 'Master of Puppets' });
    expect(p.main).toEqual({});
    expect(p.message).toContain('"Masterof Puppets"');
    expect(p.message).toContain('"Master of Puppets"');
    expect(p.message).toContain('8 tracks');
  });
  it('treats a case-only change as a rename of all, not a merge', () => {
    const p = rename(plan(ts, 'm1', 'Metalica', 'masterof puppets'));
    expect(p.merged).toBe(false);
    expect(p.group).toHaveLength(7);
  });
  it('merges into an existing positioned album, after the existing tracks', () => {
    const all = [...album('B', 'Old', 3, 2, 'o'), ...album('B', 'New', 2, 1, 'n'), mk('z', 'B', 'Z', null, 3)];
    const p = rename(plan(all, 'n1', 'B', 'Old'));
    expect(p.merged).toBe(true);
    expect(p.message).toContain('already exists (3 tracks)');
    expect(p.main).toEqual({ position: 4, album_position: 2 });
    const byId = Object.fromEntries(p.group.map((g) => [g.id, g]));
    expect(byId.n2).toEqual({ id: 'n2', revision: 'r-n2', album: 'Old', position: 5, album_position: 2 });
    expect(byId.o1).toBeUndefined();
  });
  it('a merge without any position sets none', () => {
    const all = [mk('a', 'B', 'X'), mk('b', 'B', 'Y')];
    const p = rename(plan(all, 'b', 'B', 'X'));
    expect(p.merged).toBe(true);
    expect(p.main).toEqual({});
    expect(p.group).toEqual([]);
  });
  it('a (no album) track joins an ordered album at max + 1 and its place', () => {
    const all = [...album('B', 'Alb', 3, 2), mk('solo', 'B', '')];
    expect(plan(all, 'solo', 'B', 'alb')).toEqual({ kind: 'join', position: 4, album_position: 2 });
  });
  it('a (no album) track with a leftover position still joins at max + 1', () => {
    const all = [...album('B', 'Alb', 3, 2), mk('solo', 'B', '', 9)];
    expect(plan(all, 'solo', 'B', 'alb')).toEqual({ kind: 'join', position: 4, album_position: 2 });
  });
  it('a (no album) track with a leftover position joining an album without positions has it cleared', () => {
    const all = [mk('x', 'B', 'Plain'), mk('solo', 'B', '', 9)];
    expect(plan(all, 'solo', 'B', 'plain')).toEqual({ kind: 'join', position: null });
    expect(plan(all, 'solo', 'B', 'Fresh')).toEqual({ kind: 'join', position: null });
  });
  it('a (no album) track joining a new album gets nothing', () => {
    const all = [...album('B', 'Alb', 3, 2), mk('solo', 'B', '')];
    expect(plan(all, 'solo', 'B', 'Fresh')).toEqual({ kind: 'join' });
  });
});

describe('planRename: band', () => {
  const ts = [
    ...album('Metalica', 'A1', 3, 1, 'a'), ...album('Metalica', 'A2', 2, 2, 'b'), mk('loose', 'Metalica', ''),
  ];
  it('renames every track of the band, including (no album), and keeps the order', () => {
    const p = rename(plan(ts, 'a1', 'Metallica', 'A1'));
    expect(p.scope).toBe('band');
    expect(p.count).toBe(6);
    expect(p.group).toHaveLength(5);
    expect(p.group.every((g) => g.band === 'Metallica' && g.position === undefined && g.album_position === undefined)).toBe(true);
    expect(p.message).toContain('in 2 albums');
    expect(p.merged).toBe(false);
  });
  it('a case-only band change renames everything without a merge', () => {
    const p = rename(plan(ts, 'a1', 'METALICA', 'A1'));
    expect(p.merged).toBe(false);
    expect(p.group).toHaveLength(5);
  });
  it('merges bands with a colliding album, renumbering the albums', () => {
    const all = [
      ...album('Dst', 'Same', 2, 1, 'd'), ...album('Dst', 'Own', 1, 2, 'w'),
      ...album('Src', 'same', 2, 1, 's'), ...album('Src', 'Fresh', 1, 2, 'f'),
    ];
    const p = rename(plan(all, 'f1', 'Dst', 'Fresh'));
    expect(p.merged).toBe(true);
    const by = Object.fromEntries([{ id: 'f1', ...p.main }, ...p.group].map((g) => [g.id, g]));
    // order: Same (merged) 1, Own 2, Fresh 3
    expect(by.f1.album_position).toBe(3);
    expect(by.w1).toBeUndefined();
    expect(by.d1).toBeUndefined();
    expect(by.s1).toEqual({ id: 's1', revision: 'r-s1', band: 'Dst', position: 3 }); // album_position 1 already
    expect(by.s2).toMatchObject({ band: 'Dst', position: 4 });
    expect(p.group.find((g) => g.id === 'f1')).toBeUndefined();
  });
  it('a band merge without album positions sets none', () => {
    const all = [mk('a', 'Dst', 'X'), mk('b', 'Src', 'Y'), mk('c', 'Src', '')];
    const p = rename(plan(all, 'b', 'Dst', 'Y'));
    expect(p.merged).toBe(true);
    expect(p.group.every((g) => g.album_position === undefined && g.position === undefined)).toBe(true);
    expect(p.group.map((g) => g.id)).toEqual(['c']);
  });
  it('moves a (no band) album group as a whole', () => {
    const all = [...album('', 'Orphans', 3, null, 'o'), mk('x', '', 'Other'), mk('t', 'Band', 'T')];
    const p = rename(plan(all, 'o2', 'Band', 'Orphans'));
    expect(p.count).toBe(3);
    expect(p.group.map((g) => g.id).sort()).toEqual(['o1', 'o3']);
    expect(p.group.every((g) => g.band === 'Band')).toBe(true);
  });
  it('a lone (no band) track joins without confirmation', () => {
    const all = [mk('x', '', 'Other'), mk('y', '', '')];
    expect(plan(all, 'x', 'Band', 'Other')).toEqual({ kind: 'join' });
    expect(plan(all, 'y', 'Band', '')).toEqual({ kind: 'join' });
  });
});

describe('planRename: refusals', () => {
  const ts = [mk('a', 'B', 'A')];
  it('refuses clearing and changing both', () => {
    expect(plan(ts, 'a', '', 'A')).toMatchObject({ kind: 'refused', reason: 'clear-band' });
    expect(plan(ts, 'a', 'B', ' ')).toMatchObject({ kind: 'refused', reason: 'clear-album' });
    expect(plan(ts, 'a', 'C', 'D')).toMatchObject({ kind: 'refused', reason: 'both' });
  });
  it('gives none without a change (ignoring surrounding spaces)', () => {
    expect(plan(ts, 'a', ' B ', 'A ')).toEqual({ kind: 'none' });
  });
});
