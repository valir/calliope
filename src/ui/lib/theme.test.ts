import { describe, expect, it } from 'vitest';
import { applyTheme } from './theme';

describe('applyTheme', () => {
  it('adds the dark class and sets color-scheme', () => {
    const root = document.createElement('html');
    applyTheme('dark', root);
    expect(root.classList.contains('dark')).toBe(true);
    expect(root.style.colorScheme).toBe('dark');
  });

  it('removes the dark class for light', () => {
    const root = document.createElement('html');
    root.classList.add('dark');
    applyTheme('light', root);
    expect(root.classList.contains('dark')).toBe(false);
    expect(root.style.colorScheme).toBe('light');
  });

  it('defaults to document.documentElement', () => {
    applyTheme('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    applyTheme('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });
});
