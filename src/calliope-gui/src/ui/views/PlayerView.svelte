<script lang="ts">
  import { onMount, tick } from 'svelte';
  import TrackBrowser from '../components/library/TrackBrowser.svelte';
  import PlayerPane from '../components/player/PlayerPane.svelte';
  import { load, lib, selectedTrack } from '$lib/library-state.svelte';
  import { attachPlayer, syncSelection } from '$lib/player-state.svelte';
  import { frontendLog } from '$lib/ipc';

  let searchRef = $state<HTMLInputElement | null>(null);
  let collapsed = $state(false);

  onMount(() => {
    // An unfinished Library edit keeps its data; otherwise rescan.
    if (lib.mode !== 'edit') void load();
    // After a webview reload the backend may still hold a session.
    if (!lib.selectedId) void attachPlayer();
  });

  // The player follows the selected track (shared with the Library).
  $effect(() => {
    const track = selectedTrack();
    if (lib.selectedId && !track) return;
    syncSelection(track);
  });

  async function setCollapsed(value: boolean): Promise<void> {
    if (collapsed === value) return;
    collapsed = value;
    void frontendLog(`player tree collapsed=${value}`);
    await tick();
    if (value) document.getElementById('view-heading')?.focus();
    else searchRef?.focus();
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.code === 'KeyL') {
      e.preventDefault();
      void setCollapsed(!collapsed);
      return;
    }
    if (e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && e.code === 'KeyF' && !collapsed) {
      e.preventDefault();
      searchRef?.focus();
      searchRef?.select();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<section class="flex h-full min-h-0" aria-labelledby="view-heading">
  <TrackBrowser heading="Player" bind:searchRef {collapsed} onexpand={() => void setCollapsed(false)} />
  <PlayerPane />
</section>
