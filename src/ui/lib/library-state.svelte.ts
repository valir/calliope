import { frontendLog, getRepository, listTracks, type Library, type RepoStatus } from './ipc';
import type { Draft, Mode } from './track-draft';
import { allGroupKeys, buildTree, filterTree, type BandNode } from './library-tree';

export const lib = $state({
  library: null as Library | null,
  loaded: false,
  loading: false,
  /** Load failure, shown as an alert. */
  error: '',
  /** Set when the repository folder is missing or written by a newer Calliope. */
  repoProblem: '' as string,
  expanded: {} as Record<string, boolean>,
  /** Groups the user closed while a search was showing everything expanded. */
  searchCollapsed: {} as Record<string, boolean>,
  search: '',
  selectedId: null as string | null,
  mode: 'view' as Mode,
  draft: null as Draft | null,
  busy: false,
  message: '' as string,
});

export function resetLibrary(): void {
  lib.library = null;
  lib.loaded = false;
  lib.loading = false;
  lib.error = '';
  lib.repoProblem = '';
  lib.expanded = {};
  lib.searchCollapsed = {};
  lib.search = '';
  lib.selectedId = null;
  lib.mode = 'view';
  lib.draft = null;
  lib.busy = false;
  lib.message = '';
}

function repoProblemText(status: RepoStatus, root: string): string {
  if (status === 'missing') return `The track repository folder "${root}" does not exist. Choose another one in Settings.`;
  return `The track repository at "${root}" was written by a newer Calliope. Choose another folder in Settings.`;
}

export async function load(): Promise<void> {
  lib.loading = true;
  lib.error = '';
  try {
    const info = await getRepository();
    if (info.status === 'missing' || info.status === 'newer') {
      lib.library = { root: info.root, tracks: [], problems: [] };
      lib.repoProblem = repoProblemText(info.status, info.root);
      void frontendLog(`library root=${info.root} status=${info.status} tracks=0 problems=0`);
    } else {
      lib.repoProblem = '';
      const l = await listTracks();
      lib.library = l;
      void frontendLog(
        `library root=${l.root} status=${info.status} tracks=${l.tracks.length} problems=${l.problems.length}`,
      );
      if (lib.selectedId && !l.tracks.some((t) => t.id === lib.selectedId)) lib.selectedId = null;
    }
  } catch (err) {
    lib.error = `Could not read the track library: ${String(err)}`;
    void frontendLog(`error list_tracks: ${String(err)}`);
  } finally {
    lib.loading = false;
    lib.loaded = true;
  }
}

export function fullTree(): BandNode[] {
  return buildTree(lib.library?.tracks ?? []);
}

export function visibleTree(): { tree: BandNode[]; searching: boolean; expandKeys: Set<string> } {
  const searching = lib.search.trim() !== '';
  const { tree, expandKeys } = filterTree(fullTree(), lib.search);
  return { tree, searching, expandKeys: new Set(expandKeys) };
}

export function isExpanded(key: string, searching: boolean, expandKeys: Set<string>): boolean {
  return searching ? expandKeys.has(key) && !lib.searchCollapsed[key] : !!lib.expanded[key];
}

export function toggleGroup(key: string, searching: boolean, expandKeys: Set<string>): void {
  const open = isExpanded(key, searching, expandKeys);
  if (searching) lib.searchCollapsed[key] = open;
  else lib.expanded[key] = !open;
}

export function setSearch(q: string): void {
  lib.search = q;
  lib.searchCollapsed = {};
}

export function expandAll(): void {
  const next: Record<string, boolean> = {};
  for (const k of allGroupKeys(fullTree())) next[k] = true;
  lib.expanded = next;
  lib.searchCollapsed = {};
}

export function collapseAll(): void {
  lib.expanded = {};
  if (lib.search.trim() !== '') {
    const { expandKeys } = filterTree(fullTree(), lib.search);
    lib.searchCollapsed = Object.fromEntries(expandKeys.map((k) => [k, true]));
  }
}

export function selectTrack(id: string): void {
  if (lib.selectedId === id) return;
  lib.selectedId = id;
  void frontendLog(`select id=${id}`);
}
