import type { Theme } from './ipc';
import type { ViewId } from './views';

export const ui = $state({
  view: 'library' as ViewId,
  theme: 'dark' as Theme,
  version: '',
  navCollapsed: false,
});
