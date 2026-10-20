<script lang="ts">
  import TreeToolbar from './TreeToolbar.svelte';
  import TrackTree from './TrackTree.svelte';
  import ConfirmDialog from '../ConfirmDialog.svelte';
  import LibraryIcon from '@lucide/svelte/icons/library';
  import { Button } from '$lib/components/ui/button/index.js';
  import { lib, visibleTree } from '$lib/library-state.svelte';
  import { frontendLog } from '$lib/ipc';
  import { confirmDeleteAlbum, neighbourKey } from '$lib/library-order';
  import { deleteTargetOrReason, deleteTreeTarget, logDeleteNone, type DeleteTarget } from '$lib/library-ops.svelte';
  import { confirmDeleteTrack } from '$lib/track-draft';
  import { tick } from 'svelte';

  let {
    heading,
    searchRef = $bindable(null),
    collapsed = false,
    onexpand,
    editable = false,
  }: {
    heading: string;
    searchRef?: HTMLInputElement | null;
    collapsed?: boolean;
    onexpand?: () => void;
    /** Reordering and deleting (the Library only). */
    editable?: boolean;
  } = $props();

  let tree: ReturnType<typeof TrackTree> | undefined = $state();
  let showProblems = $state(false);
  let scroller = $state<HTMLElement | null>(null);
  let confirmOpen = $state(false);
  let confirmMessage = $state('');
  let target: DeleteTarget | null = null;

  function askDelete(): void {
    const r = deleteTargetOrReason();
    if (!r.target) return logDeleteNone();
    target = r.target;
    confirmMessage =
      r.target.kind === 'track'
        ? confirmDeleteTrack(r.target.track.title)
        : confirmDeleteAlbum(r.target.bandLabel, r.target.album.label, r.target.tracks.length);
    confirmOpen = true;
  }

  async function answer(yes: boolean): Promise<void> {
    const t = target;
    target = null;
    if (!t) return;
    if (!yes) {
      void frontendLog(`delete cancelled key=${t.key}`);
      await tick();
      void tree?.focusKey(t.key);
      return;
    }
    const before = tree?.visibleKeys() ?? [];
    const { removedKeys } = await deleteTreeTarget(t);
    await tick();
    const next = neighbourKey(before, removedKeys, tree?.visibleKeys() ?? []);
    if (next) void tree?.focusKey(next);
    else searchRef?.focus();
  }

  const tracks = $derived(lib.library?.tracks ?? []);
  const problems = $derived(lib.library?.problems ?? []);
  const hasMatches = $derived(visibleTree().tree.length > 0);
  const query = $derived(lib.search.trim());
</script>

{#if collapsed}
  <div class="flex w-14 shrink-0 flex-col items-center gap-3 border-r border-border p-1 pt-4" data-testid="tree-rail">
    <h1 id="view-heading" tabindex="-1" class="sr-only">{heading}</h1>
    <Button
      variant="outline"
      class="h-[3.25rem] w-full px-0"
      aria-label="Library"
      title="Show the library (Ctrl+L)"
      onclick={() => onexpand?.()}
    >
      <LibraryIcon class="size-7" />
    </Button>
  </div>
{:else}
  <div
    bind:this={scroller}
    class="flex w-80 shrink-0 flex-col gap-3 overflow-y-auto border-r border-border p-4"
  >
    <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">{heading}</h1>
    <div class="flex flex-col gap-3 transition-opacity {lib.mode === 'edit' ? 'opacity-50' : ''}">
      <TreeToolbar bind:searchRef onsearchdown={() => tree?.focusFirst()} ondelete={editable ? askDelete : undefined} />
    </div>
    {#if lib.mode === 'edit'}
      <p role="status" class="text-sm text-muted-foreground" data-testid="tree-locked-hint">
        Finish or cancel editing to browse
      </p>
    {/if}
    {#if lib.error}
      <p role="alert" class="text-destructive">{lib.error}</p>
    {:else if lib.repoProblem}
      <p role="alert" class="text-destructive">{lib.repoProblem}</p>
    {:else if lib.loaded && tracks.length === 0}
      <p role="status" class="text-muted-foreground">
        No tracks in the repository yet.
        {#if lib.library}<span class="block break-all" data-testid="library-root">{lib.library.root}</span>{/if}
      </p>
    {:else if query !== '' && !hasMatches}
      <p role="status" class="text-muted-foreground">No tracks match "{query}"</p>
    {/if}
    <div class="flex flex-col transition-opacity {lib.mode === 'edit' ? 'opacity-50' : ''}">
      <TrackTree bind:this={tree} {editable} {scroller} ondelete={askDelete} />
    </div>
    {#if problems.length > 0}
      <div class="text-sm">
        <button
          type="button"
          class="underline underline-offset-2"
          aria-expanded={showProblems}
          aria-controls="library-problems"
          onclick={() => (showProblems = !showProblems)}
        >
          {problems.length === 1 ? '1 track folder could not be read' : `${problems.length} track folders could not be read`}
        </button>
        {#if showProblems}
          <ul id="library-problems" class="mt-2 flex flex-col gap-1 text-muted-foreground">
            {#each problems as p (p.dir)}
              <li class="break-all">{p.dir}: {p.message}</li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </div>
{/if}

<ConfirmDialog bind:open={confirmOpen} message={confirmMessage} onanswer={(yes) => void answer(yes)} />
