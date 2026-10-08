<script lang="ts">
  import { onMount } from 'svelte';
  import * as Tabs from '$lib/components/ui/tabs/index.js';
  import TrackBrowser from '../components/library/TrackBrowser.svelte';
  import { lib, load } from '$lib/library-state.svelte';
  import { TRACK_TABS } from '$lib/views';

  let searchRef = $state<HTMLInputElement | null>(null);

  onMount(() => {
    // An unfinished Library edit keeps its data; otherwise rescan.
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
  <TrackBrowser heading="Editor" bind:searchRef />
  <section aria-label="Editor" class="min-h-0 min-w-0 flex-1 overflow-y-auto p-4">
    <Tabs.Root value="stems">
      <Tabs.List>
        {#each TRACK_TABS as tab (tab.id)}
          <Tabs.Trigger value={tab.id}>{tab.label}</Tabs.Trigger>
        {/each}
      </Tabs.List>
      <Tabs.Content value="stems">
        <div data-testid="editor-pane-slot"></div>
      </Tabs.Content>
      {#each TRACK_TABS.filter((t) => t.id !== 'stems') as tab (tab.id)}
        <Tabs.Content value={tab.id}>
          <p class="mt-4 rounded-lg border border-dashed border-border p-6 text-lg" data-testid="placeholder">
            This tab will be filled by: {tab.feature}.
          </p>
        </Tabs.Content>
      {/each}
    </Tabs.Root>
  </section>
</section>
