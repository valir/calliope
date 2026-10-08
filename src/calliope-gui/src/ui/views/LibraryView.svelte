<script lang="ts">
  import { onMount } from 'svelte';
  import TrackBrowser from '../components/library/TrackBrowser.svelte';
  import TrackPane from '../components/library/TrackPane.svelte';
  import { lib, load } from '$lib/library-state.svelte';

  let searchRef = $state<HTMLInputElement | null>(null);

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
</script>

<svelte:window onkeydown={onKeydown} />

<section class="flex h-full min-h-0" aria-labelledby="view-heading">
  <TrackBrowser heading="Library" bind:searchRef />
  <section aria-label="Track" class="min-h-0 min-w-0 flex-1 p-4">
    <TrackPane />
  </section>
</section>
