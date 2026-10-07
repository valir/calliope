<script lang="ts">
  import { onMount, tick } from 'svelte';
  import NavBar from './components/NavBar.svelte';
  import StatusFooter from './components/StatusFooter.svelte';
  import LibraryView from './views/LibraryView.svelte';
  import ImportView from './views/ImportView.svelte';
  import TrackView from './views/TrackView.svelte';
  import PlaylistsView from './views/PlaylistsView.svelte';
  import PlayerView from './views/PlayerView.svelte';
  import SettingsView from './views/SettingsView.svelte';
  import { ui } from '$lib/app-state.svelte';
  import { appVersion, frontendLog } from '$lib/ipc';
  import { isNavToggleKey, viewForKey } from '$lib/views';

  let lastView = ui.view;
  $effect(() => {
    const v = ui.view;
    if (v !== lastView) {
      lastView = v;
      void frontendLog(`view=${v}`);
    }
  });

  onMount(async () => {
    try {
      ui.version = await appVersion();
    } catch (err) {
      ui.version = '';
      void frontendLog(`error app_version: ${String(err)}`);
    }
    void frontendLog(`ready view=${ui.view} theme=${ui.theme} version=${ui.version || '?'}`);
  });

  async function onKeydown(e: KeyboardEvent): Promise<void> {
    if (isNavToggleKey(e)) {
      e.preventDefault();
      ui.navCollapsed = !ui.navCollapsed;
      return;
    }
    const id = viewForKey(e);
    if (id) {
      e.preventDefault();
      ui.view = id;
      await tick();
      document.getElementById('view-heading')?.focus();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="grid h-screen grid-cols-[auto_1fr] grid-rows-[1fr_auto] bg-background text-foreground">
  <NavBar />
  <main class="overflow-auto">
    {#if ui.view === 'library'}
      <LibraryView />
    {:else if ui.view === 'import'}
      <ImportView />
    {:else if ui.view === 'track'}
      <TrackView />
    {:else if ui.view === 'playlists'}
      <PlaylistsView />
    {:else if ui.view === 'player'}
      <PlayerView />
    {:else}
      <SettingsView />
    {/if}
  </main>
  <StatusFooter />
</div>
