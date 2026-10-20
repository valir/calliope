import { describe, it, expect } from 'vitest';
import {
  albumOrderPatches, confirmDeleteAlbum, gapAt, gapToIndex, moveItem, neighbourKey, trackOrderPatches,
} from './library-order';
import { buildTree } from './library-tree';
import { FIXTURE_TRACKS } from './fixture-tracks';
import type { TrackRecord } from './ipc';

const mk = (id: string, title: string, over: Partial<TrackRecord> = {}): TrackRecord => ({
  ...FIXTURE_TRACKS[0], id, band: 'Band', album: 'Album', title, revision: `r-${id}`,
  position: null, album_position: null, ...over,
});

describe('moveItem / gapToIndex / gapAt', () => {
  it('moves up, down, to the ends and leaves from === to alone', () => {
    expect(moveItem([1, 2, 3, 4], 2, 0)).toEqual([3, 1, 2, 4]);
    expect(moveItem([1, 2, 3, 4], 0, 2)).toEqual([2, 3, 1, 4]);
    expect(moveItem([1, 2, 3, 4], 0, 3)).toEqual([2, 3, 4, 1]);
    expect(moveItem([1, 2, 3, 4], 3, 0)).toEqual([4, 1, 2, 3]);
    expect(moveItem([1, 2, 3], 1, 1)).toEqual([1, 2, 3]);
  });
  it('gapToIndex', () => {
    expect(gapToIndex(2, 0)).toBe(0);
    expect(gapToIndex(0, 3)).toBe(2);
    expect(gapToIndex(1, 1)).toBeNull();
    expect(gapToIndex(1, 2)).toBeNull();
    expect(gapToIndex(1, 3)).toBe(2);
  });
  it('gapAt uses the halves and gives null outside', () => {
    const rects = [{ top: 0, bottom: 10 }, { top: 10, bottom: 20 }, { top: 20, bottom: 30 }];
    expect(gapAt(rects, 2)).toBe(0);
    expect(gapAt(rects, 7)).toBe(1);
    expect(gapAt(rects, 12)).toBe(1);
    expect(gapAt(rects, 18)).toBe(2);
    expect(gapAt(rects, 29)).toBe(3);
    expect(gapAt(rects, -1)).toBeNull();
    expect(gapAt(rects, 30)).toBeNull();
    expect(gapAt([], 5)).toBeNull();
  });
});

describe('order patches', () => {
  it('numbers an unnumbered album when C moves above A', () => {
    const a = buildTree([mk('a', 'A'), mk('b', 'B'), mk('c', 'C')])[0].albums[0];
    expect(trackOrderPatches(a, 2, 0)).toEqual([
      { id: 'c', revision: 'r-c', position: 1 },
      { id: 'a', revision: 'r-a', position: 2 },
      { id: 'b', revision: 'r-b', position: 3 },
    ]);
  });
  it('patches only the changed tracks of a numbered album', () => {
    const a = buildTree([mk('a', 'A', { position: 1 }), mk('b', 'B', { position: 2 }), mk('c', 'C', { position: 3 })])[0]
      .albums[0];
    expect(trackOrderPatches(a, 1, 0).map((p) => p.id)).toEqual(['b', 'a']);
  });
  it('albumOrderPatches renumbers every track of all real albums, not (no album)', () => {
    const ts = [
      mk('a1', 'x', { album: 'Alpha' }), mk('a2', 'y', { album: 'Alpha' }),
      mk('b1', 'x', { album: 'Beta' }),
      mk('g1', 'x', { album: 'Gamma', position: 4 }), mk('g2', 'y', { album: 'Gamma' }),
      mk('n1', 'x', { album: '' }),
    ];
    const band = buildTree(ts)[0];
    const p = albumOrderPatches(band, 2, 0);
    expect(p.map((x) => [x.id, x.album_position])).toEqual([
      ['g1', 1], ['g2', 1], ['a1', 2], ['a2', 2], ['b1', 3],
    ]);
    expect(p.every((x) => x.position === undefined)).toBe(true);
  });
});

describe('neighbourKey / confirmDeleteAlbum', () => {
  const before = ['a', 'b', 'c', 'd'];
  it('prefers the next, then the previous, then the first, then null', () => {
    expect(neighbourKey(before, new Set(['b']), ['a', 'c', 'd'])).toBe('c');
    expect(neighbourKey(before, new Set(['d']), ['a', 'b', 'c'])).toBe('c');
    expect(neighbourKey(before, new Set(['b', 'c']), ['a', 'd'])).toBe('d');
    expect(neighbourKey(['a'], new Set(['a']), ['x'])).toBe('x');
    expect(neighbourKey(['a'], new Set(['a']), [])).toBeNull();
  });
  it('words the album confirmation', () => {
    expect(confirmDeleteAlbum('B', 'A', 1)).toContain('(1 track)');
    expect(confirmDeleteAlbum('B', 'A', 4)).toBe(
      `Delete the album "A" by B (4 tracks)? The tracks will be moved to the repository's trash folder.`,
    );
  });
});
