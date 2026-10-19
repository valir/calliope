<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { Progress } from '$lib/components/ui/progress/index.js';
  import ExtractProgress from './ExtractProgress.svelte';
  import { cancelRunning, imp } from '$lib/import-state.svelte';

  const job = $derived(imp.job);
  const batch = $derived(job?.batch ?? null);
  const index = $derived(batch?.current ?? null);
  const total = $derived(batch?.files.length ?? 0);
  const name = $derived(batch && index !== null ? (batch.files[index] ?? '') : '');
  const preparing = $derived(job?.phase === 'preparing');
  const items = $derived(batch?.items ?? []);
</script>

<section class="flex max-w-xl flex-col gap-4" aria-label="Directory import progress">
  <h3 class="text-lg font-medium">Importing a directory</h3>

  {#if index !== null}
    <p role="status" data-testid="batch-current" class="text-base">File {index + 1} of {total}: {name}</p>
  {/if}

  {#if preparing}
    <div class="flex flex-col gap-2" role="status">
      <span>Preparing audio...</span>
      <Progress value={null} aria-label="Preparing audio" />
    </div>
  {:else}
    <ExtractProgress embedded />
  {/if}

  <div><Button variant="outline" onclick={() => void cancelRunning()}>Cancel</Button></div>

  <div class="flex flex-col gap-1">
    <h4 class="text-sm font-medium">Done</h4>
    <ul aria-label="Files done" class="max-h-48 overflow-y-auto text-sm">
      {#each items as it (it.index)}
        {#if it.result === 'saved'}
          <li>
            {it.name}: saved as {it.track.title}{#if it.dropped.length > 0}{' '}(dropped empty stems: {it.dropped.map((s) => s.name).join(', ')}){/if}
          </li>
        {:else}
          <li class="text-amber-700 dark:text-primary">{it.name}: failed: {it.message}</li>
        {/if}
      {/each}
    </ul>
  </div>
</section>
