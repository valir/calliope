<script lang="ts">
  import { onMount } from 'svelte';
  import SourcePage from '../components/import/SourcePage.svelte';
  import ImportEditPane from '../components/import/ImportEditPane.svelte';
  import ExtractProgress from '../components/import/ExtractProgress.svelte';
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
      <ExtractProgress />
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
