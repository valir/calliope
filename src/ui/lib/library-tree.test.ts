import { describe, it, expect } from 'vitest';
import { buildTree, allGroupKeys, filterTree } from './library-tree';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { TrackRecord } from './ipc';

const tree = buildTree(FIXTURE_TRACKS);
const titles = (q: string) =>
  filterTree(tree, q).tree.flatMap((b) => b.albums.flatMap((a) => a.tracks.map((t) => t.title)));

describe('buildTree', () => {
  it('orders bands, with (no band) last', () => {
    expect(tree.map((b) => b.label)).toEqual([
      'Amber Fields',
      'the Night Owls',
      'Zephyr Lane',
      '(no band)',
    ]);
  });
  it('orders albums and tracks case-insensitively', () => {
    const amber = tree[0];
    expect(amber.albums.map((a) => a.label)).toEqual(['Copper Sky', 'Northern Roads']);
    expect(amber.albums[1].tracks.map((t) => t.title)).toEqual(['after midnight', 'Slow Burn']);
  });
  it('puts (no album) last and uses normalized keys', () => {
    const zephyr = tree[2];
    expect(zephyr.albums).toHaveLength(1);
    expect(zephyr.albums[0].label).toBe('(no album)');
    expect(zephyr.albums[0].key).toBe('b:zephyr lane|a:');
    expect(tree[0].key).toBe('b:amber fields');
    expect(tree[3].key).toBe('b:');
  });
  it('groups spelling variants and keeps the first in sort order', () => {
    const mk = (id: string, band: string): TrackRecord => ({
      ...FIXTURE_TRACKS[0], id, band: band, album: ' Roads ', title: id,
    });
    const t = buildTree([mk('1', 'amber fields'), mk('2', 'Amber Fields  '.trimEnd()), mk('3', ' Amber Fields')]);
    expect(t).toHaveLength(1);
    expect(t[0].albums[0].tracks).toHaveLength(3);
    expect(t[0].label).toBe('Amber Fields');
    expect(t[0].albums[0].label).toBe('Roads');
  });
  it('sorts numerically and breaks ties by id', () => {
    const mk = (id: string, title: string): TrackRecord => ({ ...FIXTURE_TRACKS[0], id, title });
    const t = buildTree([mk('b', 'Song 10'), mk('a', 'Song 2'), mk('d', 'same'), mk('c', 'same')]);
    expect(t[0].albums[0].tracks.map((x) => x.id)).toEqual(['c', 'd', 'a', 'b']);
  });
  it('empty library gives an empty tree', () => expect(buildTree([])).toEqual([]));
});

describe('allGroupKeys', () => {
  it('lists band and album keys', () => {
    expect(allGroupKeys(tree)).toEqual([
      'b:amber fields',
      'b:amber fields|a:copper sky',
      'b:amber fields|a:northern roads',
      'b:the night owls',
      'b:the night owls|a:lanterns',
      'b:zephyr lane',
      'b:zephyr lane|a:',
      'b:',
      'b:|a:',
    ]);
  });
});

describe('filterTree', () => {
  it('empty and whitespace-only queries return the full tree and no expansion', () => {
    for (const q of ['', '   ']) {
      const r = filterTree(tree, q);
      expect(r.tree).toEqual(tree);
      expect(r.expandKeys).toEqual([]);
    }
  });
  it('nght finds the Night Owls', () => expect(titles('nght')).toEqual(['Lanterns']));
  it('brn finds Slow Burn', () => expect(titles('brn')).toEqual(['Slow Burn']));
  it('ber finds Amber Fields tracks', () =>
    expect(titles('ber')).toEqual(['Copper Sky', 'after midnight', 'Slow Burn']));
  it('mbr finds nothing', () => {
    const r = filterTree(tree, 'mbr');
    expect(r.tree).toEqual([]);
    expect(r.expandKeys).toEqual([]);
  });
  it('owls lan finds Lanterns', () => expect(titles('owls lan')).toEqual(['Lanterns']));
  it('cafe finds Café, under (no band)', () => {
    const r = filterTree(tree, 'cafe');
    expect(r.tree.map((b) => b.label)).toEqual(['(no band)']);
    expect(titles('CAFÉ')).toEqual(['Café Practice Groove']);
  });
  it('expands every branch with a match', () => {
    const r = filterTree(tree, 'slow');
    expect(r.expandKeys).toEqual(['b:amber fields', 'b:amber fields|a:northern roads']);
    expect(r.tree[0].albums).toHaveLength(1);
  });
});
