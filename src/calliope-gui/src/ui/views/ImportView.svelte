<script lang="ts">
  import { onMount } from 'svelte';
  import SourcePage from '../components/import/SourcePage.svelte';
  import ImportEditPane from '../components/import/ImportEditPane.svelte';
  import ExtractProgress from '../components/import/ExtractProgress.svelte';
  import BatchProgress from '../components/import/BatchProgress.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import {
    attach,
    backToMenu,
    imp,
    importAnother,
    jobRunning,
    openStemExtraction,
    showInLibrary,
    step,
  } from '$lib/import-state.svelte';

  const current = $derived(step());
  const track = $derived(imp.job?.track ?? null);
  const dropped = $derived(imp.job?.dropped ?? []);
  const batch = $derived(imp.job?.batch ?? null);
  const batchSaved = $derived(
    (batch?.items ?? []).flatMap((i) => (i.result === 'saved' ? [i] : [])),
  );
  const batchFailed = $derived(
    (batch?.items ?? []).flatMap((i) => (i.result === 'failed' ? [i] : [])),
  );

  // A job may be running or waiting for the user while this view was away: re-attach.
  onMount(() => void attach());
</script>

<section class="flex flex-col gap-4 p-8" aria-labelledby="view-heading">
  <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">Import</h1>

  {#if current === 'menu'}
    <p class="text-lg text-muted-foreground">Choose what to import.</p>
    <div>
      <Button size="lg" class="h-12 px-6 text-base" onclick={openStemExtraction}>Stem Extraction</Button>
    </div>
  {:else}
    <div class="flex items-center gap-3">
      <Button variant="outline" size="sm" disabled={jobRunning() || current !== 'source'} onclick={backToMenu}>Back</Button>
      <h2 class="text-xl font-medium">Stem Extraction</h2>
    </div>

    {#if current === 'source'}
      <SourcePage />
    {:else if current === 'edit'}
      <ImportEditPane />
    {:else if current === 'extract'}
      {#if batch}<BatchProgress />{:else}<ExtractProgress />{/if}
    {:else if current === 'done' && batch}
      <section class="flex max-w-xl flex-col gap-4" aria-label="Import finished">
        <p role="status" class="text-lg">
          Imported {batchSaved.length} of {batch.files.length} files from {batch.dir}.
        </p>
        {#if batchSaved.length > 0}
          <div class="flex flex-col gap-1">
            <h3 class="text-base font-medium">Saved tracks</h3>
            <ul aria-label="Saved tracks" class="list-disc pl-6">
              {#each batchSaved as it (it.index)}
                <li>
                  {it.track.title}{#if it.dropped.length > 0}: dropped empty stems: {it.dropped.map((s) => s.name).join(', ')}{/if}
                </li>
              {/each}
            </ul>
          </div>
        {/if}
        {#if batchFailed.length > 0}
          <div class="flex flex-col gap-1">
            <h3 class="text-base font-medium">Failed files</h3>
            <ul aria-label="Failed files" class="list-disc pl-6 text-amber-700 dark:text-primary">
              {#each batchFailed as it (it.index)}
                <li>{it.name}: {it.message}</li>
              {/each}
            </ul>
          </div>
        {/if}
        <div class="flex gap-2">
          <Button onclick={showInLibrary}>Show in Library</Button>
          <Button variant="outline" onclick={importAnother}>Import another track</Button>
        </div>
      </section>
    {:else if current === 'done' && track}
      <section class="flex max-w-xl flex-col gap-4" aria-label="Import finished">
        <p role="status" class="text-lg">Saved {track.title} with {track.stems.length} stems.</p>
        {#if dropped.length > 0}
          <p class="text-lg">Dropped empty stems: {dropped.map((s) => s.name).join(', ')}.</p>
        {/if}
        <div class="flex gap-2">
          <Button onclick={showInLibrary}>Show in Library</Button>
          <Button variant="outline" onclick={importAnother}>Import another track</Button>
        </div>
      </section>
    {/if}
  {/if}
</section>
