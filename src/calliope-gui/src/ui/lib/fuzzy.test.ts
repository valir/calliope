import { describe, it, expect } from 'vitest';
import { normalize, tokenMatches, trackMatches } from './fuzzy';

const t = (band: string, album: string, title: string) => ({ band, album, title });

describe('normalize', () => {
  it('lower-cases and strips diacritics', () => {
    expect(normalize('Café')).toBe('cafe');
    expect(normalize('ÀÉÎõü Ñ')).toBe('aeiou n');
  });
});

describe('tokenMatches', () => {
  const f = normalize;
  it('substring', () => expect(tokenMatches('ber', f('Amber Fields'))).toBe(true));
  it('word-start subsequence', () => {
    expect(tokenMatches('nght', f('the Night Owls'))).toBe(true);
    expect(tokenMatches('brn', f('Slow Burn'))).toBe(true);
  });
  it('subsequence must start at a word start', () => {
    expect(tokenMatches('mbr', f('Amber Fields'))).toBe(false);
  });
  it('order matters and every char must be found', () => {
    expect(tokenMatches('nrb', f('Slow Burn'))).toBe(false);
    expect(tokenMatches('bnn', f('Slow Burn'))).toBe(false);
  });
  it('later word start can host the subsequence', () => {
    expect(tokenMatches('bn', f('burn'))).toBe(true);
    expect(tokenMatches('bsr', f('so burn'))).toBe(false);
    expect(tokenMatches('bu', f('abc bxu'))).toBe(true);
  });
});

describe('trackMatches', () => {
  it('matches on band, album or title', () => {
    expect(trackMatches(t('Zephyr Lane', '', 'Open Water'), 'zeph')).toBe(true);
    expect(trackMatches(t('X', 'Copper Sky', 'Y'), 'copper')).toBe(true);
    expect(trackMatches(t('X', 'Y', 'Open Water'), 'water')).toBe(true);
  });
  it('requires every token, each in any field', () => {
    const lanterns = t('the Night Owls', 'Lanterns', 'Lanterns');
    expect(trackMatches(lanterns, 'owls lan')).toBe(true);
    expect(trackMatches(lanterns, 'owls zzz')).toBe(false);
  });
  it('is case and accent insensitive in both directions', () => {
    const cafe = t('', '', 'Café Practice Groove');
    expect(trackMatches(cafe, 'cafe')).toBe(true);
    expect(trackMatches(cafe, 'CAFÉ')).toBe(true);
    expect(trackMatches(t('', '', 'Cafe'), 'café')).toBe(true);
  });
  it('empty or whitespace query matches all', () => {
    expect(trackMatches(t('a', 'b', 'c'), '')).toBe(true);
    expect(trackMatches(t('a', 'b', 'c'), '  \t ')).toBe(true);
  });
});
