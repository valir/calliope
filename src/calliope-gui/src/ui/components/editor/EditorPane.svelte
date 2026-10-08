<script lang="ts">
  import { Progress } from '$lib/components/ui/progress/index.js';
  import { ed } from '$lib/editor-state.svelte';
  import MixLane from './MixLane.svelte';
  import StemLane from './StemLane.svelte';
</script>

<div class="flex h-full min-h-0 flex-col gap-4" data-testid="editor-pane">
  <div class="min-h-0 flex-1 overflow-y-auto overflow-x-hidden" data-testid="lanes">
    {#if ed.status === 'active'}
      <div class="flex flex-col gap-3">
        {#each ed.stems as lane (lane.name)}
          <StemLane {lane} />
        {/each}
      </div>
    {:else if ed.status === 'loading'}
      <div class="flex flex-col gap-2 p-2" role="status">
        <p class="text-lg">
          Loading stems{#if ed.loading} ({ed.loading.done} of {ed.loading.total}){/if}...
        </p>
        <Progress
          value={ed.loading && ed.loading.total ? (ed.loading.done / ed.loading.total) * 100 : null}
          aria-label="Loading stems"
        />
      </div>
    {:else if ed.status === 'error'}
      <p class="rounded-lg border border-destructive p-4 text-lg text-destructive" role="alert">{ed.message}</p>
    {:else}
      <p class="p-2 text-lg text-muted-foreground" data-testid="editor-inactive">{ed.message}</p>
    {/if}
  </div>
  <MixLane />
</div>
