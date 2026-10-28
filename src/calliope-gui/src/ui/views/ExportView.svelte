<script lang="ts">
  import { onMount } from 'svelte';
  import TrackBrowser from '../components/library/TrackBrowser.svelte';
  import ExportPane from '../components/export/ExportPane.svelte';
  import { lib, load } from '$lib/library-state.svelte';
  import { attachExport, beginExportVisit } from '$lib/export-state.svelte';

  let searchRef = $state<HTMLInputElement | null>(null);

  onMount(() => {
    // An unfinished Library edit keeps its data; otherwise rescan.
    if (lib.mode !== 'edit') void load();
    beginExportVisit();
    void attachExport();
  });

  function onKeydown(e: KeyboardEvent): void {
    if (e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.code === 'KeyF') {
      e.preventDefault();
      searchRef?.focus();
      searchRef?.select();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<section class="flex h-full min-h-0" aria-labelledby="view-heading">
  <TrackBrowser heading="Export" bind:searchRef />
  <section aria-label="Export" class="flex min-h-0 min-w-0 flex-1 flex-col overflow-auto p-4">
    <ExportPane />
  </section>
</section>
