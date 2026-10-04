import type { Theme } from './ipc';

export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
  root.classList.toggle('dark', theme === 'dark');
  root.style.colorScheme = theme;
}
