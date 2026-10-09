<script lang="ts">
  import { onMount } from 'svelte';
  import * as Tabs from '$lib/components/ui/tabs/index.js';
  import TrackBrowser from '../components/library/TrackBrowser.svelte';
  import EditorPane from '../components/editor/EditorPane.svelte';
  import { lib, load, selectedTrack } from '$lib/library-state.svelte';
  import { attachEditor, save, syncSelection, togglePlay } from '$lib/editor-state.svelte';
  import { TRACK_TABS } from '$lib/views';

  let searchRef = $state<HTMLInputElement | null>(null);

  onMount(() => {
    // An unfinished Library edit keeps its data; otherwise rescan.
    if (lib.mode !== 'edit') void load();
    // After a webview reload the backend may still hold a session. With a track selected
    // the editor opens that one instead, and a late attach must not take its event sink.
    if (!lib.selectedId) void attachEditor();
  });

  // The editor follows the selected track (shared with the Library).
  $effect(() => {
    const track = selectedTrack();
    // A selection whose record is not loaded yet is not "no selection".
    if (lib.selectedId && !track) return;
    syncSelection(track);
  });

  const OWN_SPACE = 'input, textarea, select, button, a[href], [role="checkbox"], [role="slider"], [role="treeitem"], [role="tab"], [contenteditable="true"]';

  function onKeydown(e: KeyboardEvent): void {
    if (e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.code === 'KeyS') {
      e.preventDefault();
      void save();
      return;
    }
    if (e.code === 'Space' && !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey && !e.defaultPrevented) {
      const t = e.target;
      if (t instanceof Element && t.closest(OWN_SPACE)) return;
      e.preventDefault();
      void togglePlay();
      return;
    }
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
  <section aria-label="Editor" class="flex min-h-0 min-w-0 flex-1 flex-col p-4">
    <Tabs.Root value="stems" class="min-h-0 flex-1">
      <Tabs.List>
        {#each TRACK_TABS as tab (tab.id)}
          <Tabs.Trigger value={tab.id}>{tab.label}</Tabs.Trigger>
        {/each}
      </Tabs.List>
      <Tabs.Content value="stems" class="min-h-0 flex-1">
        <EditorPane />
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
