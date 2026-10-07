<script lang="ts">
  import { onMount } from 'svelte';
  import TreeToolbar from '../components/library/TreeToolbar.svelte';
  import TrackTree from '../components/library/TrackTree.svelte';
  import TrackPane from '../components/library/TrackPane.svelte';
  import { lib, load, visibleTree } from '$lib/library-state.svelte';

  let searchRef = $state<HTMLInputElement | null>(null);
  let tree: ReturnType<typeof TrackTree> | undefined = $state();
  let showProblems = $state(false);

  onMount(() => {
    // An unfinished edit keeps its data; otherwise rescan.
    if (lib.mode !== 'edit') void load();
  });

  function onKeydown(e: KeyboardEvent): void {
    if (e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.code === 'KeyF') {
      e.preventDefault();
      searchRef?.focus();
      searchRef?.select();
    }
  }

  const tracks = $derived(lib.library?.tracks ?? []);
  const problems = $derived(lib.library?.problems ?? []);
  const hasMatches = $derived(visibleTree().tree.length > 0);
  const query = $derived(lib.search.trim());
</script>

<svelte:window onkeydown={onKeydown} />

<section class="flex h-full min-h-0" aria-labelledby="view-heading">
  <div class="flex w-80 shrink-0 flex-col gap-3 overflow-y-auto border-r border-border p-4">
    <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">Library</h1>
    <div class="flex flex-col gap-3 transition-opacity {lib.mode === 'edit' ? 'opacity-50' : ''}">
      <TreeToolbar bind:searchRef onsearchdown={() => tree?.focusFirst()} />
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
      <TrackTree bind:this={tree} />
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
  <section aria-label="Track" class="min-h-0 min-w-0 flex-1 p-4">
    <TrackPane />
  </section>
</section>
