<script lang="ts">
  import { tick } from 'svelte';
  import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import { cn } from '$lib/utils';
  import type { TrackRecord } from '$lib/ipc';
  import {
    lib,
    visibleTree,
    isExpanded,
    toggleGroup,
    selectTrack,
  } from '$lib/library-state.svelte';

  interface Item {
    key: string;
    level: 1 | 2 | 3;
    label: string;
    group: boolean;
    expanded: boolean;
    parent: string | null;
    track?: TrackRecord;
  }

  const view = $derived(visibleTree());
  const items = $derived.by<Item[]>(() => {
    const { tree, searching, expandKeys } = view;
    const out: Item[] = [];
    for (const band of tree) {
      const bOpen = isExpanded(band.key, searching, expandKeys);
      out.push({ key: band.key, level: 1, label: band.label, group: true, expanded: bOpen, parent: null });
      if (!bOpen) continue;
      for (const album of band.albums) {
        const aOpen = isExpanded(album.key, searching, expandKeys);
        out.push({ key: album.key, level: 2, label: album.label, group: true, expanded: aOpen, parent: band.key });
        if (!aOpen) continue;
        for (const t of album.tracks) {
          out.push({ key: `t:${t.id}`, level: 3, label: t.title, group: false, expanded: false, parent: album.key, track: t });
        }
      }
    }
    return out;
  });

  let focusKey = $state<string | null>(null);
  let root: HTMLDivElement | undefined = $state();

  // Roving tabindex: exactly one item is tabbable.
  const tabbableKey = $derived(
    items.find((i) => i.key === focusKey)?.key ??
      items.find((i) => i.track && i.track.id === lib.selectedId)?.key ??
      items[0]?.key ??
      null,
  );

  async function focusItem(key: string): Promise<void> {
    focusKey = key;
    await tick();
    root?.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`)?.focus();
  }

  export function focusFirst(): void {
    if (items[0]) void focusItem(tabbableKey ?? items[0].key);
  }

  function activate(item: Item): void {
    if (item.group) toggleGroup(item.key, view.searching, view.expandKeys);
    else if (item.track) selectTrack(item.track.id);
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const idx = items.findIndex((i) => i.key === (document.activeElement as HTMLElement | null)?.dataset.key);
    if (idx < 0) return;
    const item = items[idx];
    let handled = true;
    switch (e.key) {
      case 'ArrowDown':
        if (idx + 1 < items.length) void focusItem(items[idx + 1].key);
        break;
      case 'ArrowUp':
        if (idx > 0) void focusItem(items[idx - 1].key);
        break;
      case 'Home':
        void focusItem(items[0].key);
        break;
      case 'End':
        void focusItem(items[items.length - 1].key);
        break;
      case 'ArrowRight':
        if (item.group && !item.expanded) toggleGroup(item.key, view.searching, view.expandKeys);
        else if (item.group && items[idx + 1]?.parent === item.key) void focusItem(items[idx + 1].key);
        break;
      case 'ArrowLeft':
        if (item.group && item.expanded) toggleGroup(item.key, view.searching, view.expandKeys);
        else if (item.parent) void focusItem(item.parent);
        break;
      case 'Enter':
      case ' ':
        activate(item);
        break;
      default:
        handled = false;
    }
    if (handled) e.preventDefault();
  }

  const pad = { 1: 'pl-2', 2: 'pl-6', 3: 'pl-10' } as const;
</script>

<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
  bind:this={root}
  role="tree"
  aria-label="Tracks"
  tabindex="-1"
  class="flex flex-col"
  inert={lib.mode === 'edit'}
  {onkeydown}
>
  {#each items as item (item.key)}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      role="treeitem"
      aria-level={item.level}
      aria-expanded={item.group ? item.expanded : undefined}
      aria-selected={item.track ? item.track.id === lib.selectedId : undefined}
      tabindex={item.key === tabbableKey ? 0 : -1}
      data-key={item.key}
      class={cn(
        'flex cursor-pointer items-center gap-1 rounded-md py-1 pr-2 text-base outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50',
        pad[item.level],
        item.group && 'font-medium',
        item.track && item.track.id === lib.selectedId && 'bg-primary/15 text-foreground shadow-[inset_3px_0_0_var(--primary)]',
      )}
      onclick={() => {
        focusKey = item.key;
        activate(item);
      }}
      onfocus={() => (focusKey = item.key)}
    >
      {#if item.group}
        {#if item.expanded}
          <ChevronDownIcon class="size-4 shrink-0" aria-hidden="true" />
        {:else}
          <ChevronRightIcon class="size-4 shrink-0" aria-hidden="true" />
        {/if}
      {:else}
        <span class="size-4 shrink-0"></span>
      {/if}
      {#if item.track}
        {#if item.track.type === 'stem'}
          <span
            role="img"
            aria-label="Stem track"
            data-track-type="stem"
            class="inline-flex size-5 shrink-0 items-center justify-center rounded border border-primary text-xs font-semibold leading-none text-primary"
            >S</span
          >
        {:else}
          <span
            role="img"
            aria-label="Backing track"
            data-track-type="backing"
            class="inline-flex size-5 shrink-0 items-center justify-center rounded border border-border text-xs font-semibold leading-none text-muted-foreground"
            >B</span
          >
        {/if}
      {/if}
      <span class="truncate">{item.label}</span>
    </div>
  {/each}
</div>
