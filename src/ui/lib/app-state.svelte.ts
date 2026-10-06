import { checkEdgeAi, type EdgeAiState, type Theme } from './ipc';
import type { ViewId } from './views';

export const ui = $state({
  view: 'library' as ViewId,
  theme: 'dark' as Theme,
  version: '',
  navCollapsed: false,
  /** Edge-AI status for the footer; refreshed at start-up and after Settings changes. */
  edgeAi: 'not-configured' as EdgeAiState,
  /** Set by "Change in Settings"; the Stem extraction card scrolls to itself and clears it. */
  settingsTarget: '' as '' | 'stem',
});

export function applyEdgeAi(state: EdgeAiState): void {
  ui.edgeAi = state;
}

export async function refreshEdgeAi(): Promise<void> {
  try {
    ui.edgeAi = (await checkEdgeAi()).state;
  } catch {
    ui.edgeAi = 'unreachable';
  }
}
