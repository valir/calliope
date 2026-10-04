import { describe, it, expect } from 'vitest';
import {
  VIEWS,
  TRACK_TABS,
  SETTINGS_PLACEHOLDERS,
  featureText,
  viewForKey,
  isNavToggleKey,
  type KeyLike,
} from './views';

const key = (code: string, o: Partial<KeyLike> = {}): KeyLike => ({
  code,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...o,
});

describe('views', () => {
  it('has labels in order', () => {
    expect(VIEWS.map((v) => v.label)).toEqual(['Library', 'Import', 'Track', 'Playlists', 'Player', 'Settings']);
  });
  it('has digits 1..6', () => {
    expect(VIEWS.map((v) => v.digit)).toEqual([1, 2, 3, 4, 5, 6]);
  });
  it('has features for all but playlists', () => {
    for (const v of VIEWS) {
      if (v.id === 'playlists') expect(v.features).toHaveLength(0);
      else expect(v.features.length).toBeGreaterThan(0);
    }
  });
  it('featureText', () => {
    expect(featureText([])).toBe('Planned; there is no feature spec yet.');
    expect(featureText(['a', 'b'])).toBe('This view will be filled by: a, b.');
  });
  it('viewForKey maps Alt+digit', () => {
    expect(viewForKey(key('Digit1', { altKey: true }))).toBe('library');
    expect(viewForKey(key('Digit6', { altKey: true }))).toBe('settings');
    expect(viewForKey(key('Numpad3', { altKey: true }))).toBe('track');
    expect(viewForKey(key('Digit4', { altKey: true }))).toBe('playlists');
    expect(viewForKey(key('Digit5', { altKey: true }))).toBe('player');
  });
  it('viewForKey rejects other keys', () => {
    expect(viewForKey(key('Digit7', { altKey: true }))).toBeNull();
    expect(viewForKey(key('Digit0', { altKey: true }))).toBeNull();
    expect(viewForKey(key('KeyA', { altKey: true }))).toBeNull();
  });
  it('viewForKey rejects extra or missing modifiers', () => {
    expect(viewForKey(key('Digit1', { altKey: true, ctrlKey: true }))).toBeNull();
    expect(viewForKey(key('Digit1', { altKey: true, shiftKey: true }))).toBeNull();
    expect(viewForKey(key('Digit1', { altKey: true, metaKey: true }))).toBeNull();
    expect(viewForKey(key('Digit1'))).toBeNull();
  });
  it('isNavToggleKey', () => {
    expect(isNavToggleKey(key('KeyB', { ctrlKey: true }))).toBe(true);
    expect(isNavToggleKey(key('KeyB', { ctrlKey: true, altKey: true }))).toBe(false);
    expect(isNavToggleKey(key('KeyC', { ctrlKey: true }))).toBe(false);
  });
  it('has tabs and settings placeholders', () => {
    expect(TRACK_TABS).toHaveLength(5);
    expect(SETTINGS_PLACEHOLDERS).toHaveLength(3);
  });
});
