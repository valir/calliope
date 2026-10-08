<script lang="ts">
  import LibraryIcon from '@lucide/svelte/icons/library';
  import FileInputIcon from '@lucide/svelte/icons/file-input';
  import AudioWaveformIcon from '@lucide/svelte/icons/audio-waveform';
  import ListMusicIcon from '@lucide/svelte/icons/list-music';
  import PlayIcon from '@lucide/svelte/icons/play';
  import SettingsIcon from '@lucide/svelte/icons/settings';
  import PanelLeftIcon from '@lucide/svelte/icons/panel-left';
  import type { Component } from 'svelte';
  import { ui } from '$lib/app-state.svelte';
  import { VIEWS, shortcutLabel, type ViewId } from '$lib/views';
  import { cn } from '$lib/utils';

  const icons: Record<ViewId, Component> = {
    library: LibraryIcon,
    import: FileInputIcon,
    editor: AudioWaveformIcon,
    playlists: ListMusicIcon,
    player: PlayIcon,
    settings: SettingsIcon,
  };
</script>

<nav
  aria-label="Main"
  class={cn(
    'flex flex-col gap-1 border-r border-sidebar-border bg-sidebar p-2 text-sidebar-foreground',
    ui.navCollapsed ? 'w-14' : 'w-56'
  )}
>
  <button
    type="button"
    class="mb-2 flex h-10 items-center gap-3 rounded-md px-3 text-sm hover:bg-sidebar-accent"
    aria-expanded={!ui.navCollapsed}
    aria-label="Collapse navigation"
    title="Collapse navigation (Ctrl+B)"
    onclick={() => (ui.navCollapsed = !ui.navCollapsed)}
  >
    <PanelLeftIcon class="size-5 shrink-0" />
    {#if !ui.navCollapsed}<span class="text-muted-foreground">Ctrl+B</span>{/if}
  </button>
  {#each VIEWS as view (view.id)}
    {@const Icon = icons[view.id]}
    {@const active = ui.view === view.id}
    <button
      type="button"
      class={cn(
        'flex h-11 items-center gap-3 rounded-md border-l-4 px-2 text-base hover:bg-sidebar-accent',
        active
          ? 'border-sidebar-primary bg-sidebar-accent text-sidebar-accent-foreground'
          : 'border-transparent'
      )}
      aria-current={active ? 'page' : undefined}
      aria-label={view.label}
      title={`${view.label} (${shortcutLabel(view)})`}
      onclick={() => (ui.view = view.id)}
    >
      <Icon class="size-5 shrink-0" />
      {#if !ui.navCollapsed}
        <span class="flex-1 text-left">{view.label}</span>
        <span class="text-xs text-muted-foreground">{shortcutLabel(view)}</span>
      {/if}
    </button>
  {/each}
</nav>
