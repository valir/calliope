<script lang="ts">
  import TreeToolbar from './TreeToolbar.svelte';
  import TrackTree from './TrackTree.svelte';
  import { lib, visibleTree } from '$lib/library-state.svelte';

  let {
    heading,
    searchRef = $bindable(null),
  }: { heading: string; searchRef?: HTMLInputElement | null } = $props();

  let tree: ReturnType<typeof TrackTree> | undefined = $state();
  let showProblems = $state(false);

  const tracks = $derived(lib.library?.tracks ?? []);
  const problems = $derived(lib.library?.problems ?? []);
  const hasMatches = $derived(visibleTree().tree.length > 0);
  const query = $derived(lib.search.trim());
</script>

  <div class="flex w-80 shrink-0 flex-col gap-3 overflow-y-auto border-r border-border p-4">
    <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">{heading}</h1>
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
