<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import { cn } from '$lib/utils';
  import { frontendLog, type TrackRecord } from '$lib/ipc';
  import { gapAt } from '$lib/library-order';
  import { dropTreeItem, moveTreeItem, reorderAllowed } from '$lib/library-ops.svelte';
  import { trackLabel } from '$lib/library-tree';
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
    /** A track of a real album or a real album row: the items that can be reordered. */
    movable: boolean;
  }

  let {
    editable = false,
    scroller = null,
    ondelete,
  }: { editable?: boolean; scroller?: HTMLElement | null; ondelete?: () => void } = $props();

  const view = $derived(visibleTree());
  const items = $derived.by<Item[]>(() => {
    const { tree, searching, expandKeys } = view;
    const out: Item[] = [];
    for (const band of tree) {
      const bOpen = isExpanded(band.key, searching, expandKeys);
      out.push({ key: band.key, level: 1, label: band.label, group: true, expanded: bOpen, parent: null, movable: false });
      if (!bOpen) continue;
      for (const album of band.albums) {
        const aOpen = isExpanded(album.key, searching, expandKeys);
        out.push({ key: album.key, level: 2, label: album.label, group: true, expanded: aOpen, parent: band.key, movable: !album.isNone });
        if (!aOpen) continue;
        for (const t of album.tracks) {
          out.push({ key: `t:${t.id}`, level: 3, label: trackLabel(t, album), group: false, expanded: false, parent: album.key, track: t, movable: !album.isNone });
        }
      }
    }
    return out;
  });

  let root: HTMLDivElement | undefined = $state();

  // Roving tabindex: exactly one item is tabbable.
  const tabbableKey = $derived(
    items.find((i) => i.key === lib.treeFocus)?.key ??
      items.find((i) => i.track && i.track.id === lib.selectedId)?.key ??
      items[0]?.key ??
      null,
  );

  async function focusItem(key: string): Promise<void> {
    lib.treeFocus = key;
    await tick();
    root?.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`)?.focus();
  }

  export function focusKey(key: string): Promise<void> {
    return focusItem(key);
  }

  /** The keys of the rows shown now, top to bottom. */
  export function visibleKeys(): string[] {
    return items.map((i) => i.key);
  }

  export function focusFirst(): void {
    if (items[0]) void focusItem(tabbableKey ?? items[0].key);
  }

  function activate(item: Item): void {
    if (item.group) toggleGroup(item.key, view.searching, view.expandKeys);
    else if (item.track) selectTrack(item.track.id);
  }

  function onkeydown(e: KeyboardEvent): void {
    if (editable && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
      const active = (document.activeElement as HTMLElement | null)?.dataset.key;
      if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
        e.preventDefault();
        if (active && items.some((i) => i.key === active)) {
          void moveTreeItem(active, e.key === 'ArrowUp' ? -1 : 1, 'key').then(() => focusItem(active));
        }
        return;
      }
      if (!e.altKey && e.key === 'Delete') {
        e.preventDefault();
        ondelete?.();
        return;
      }
    }
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

  // ---- pointer drag and drop (editable trees only) ----
  const DRAG_THRESHOLD = 5;
  const EDGE = 24;
  const SCROLL_STEP = 8;

  interface Sibling {
    /** The row that gets the "before" line. */
    key: string;
    /** The row that gets the "after" line when this is the last sibling. */
    endKey: string;
    top: number;
    bottom: number;
  }

  let press: { key: string; startY: number } | null = null;
  let dragging = false;
  let lastEvent: { x: number; y: number } | null = null;
  let loggedTarget: string | null = null;
  let frame = 0;
  let swallowClick = false;
  let dragKey = $state<string | null>(null);
  let drop = $state<{ key: string; where: 'before' | 'after' } | null>(null);
  let dropGap: number | null = null;

  const rowOf = (key: string): HTMLElement | null =>
    root?.querySelector<HTMLElement>(`[data-key="${CSS.escape(key)}"]`) ?? null;

  function siblings(key: string): Sibling[] {
    const idx = items.findIndex((i) => i.key === key);
    if (idx < 0) return [];
    const me = items[idx];
    const out: Sibling[] = [];
    const rect = (k: string) => rowOf(k)?.getBoundingClientRect();
    for (let i = 0; i < items.length; i++) {
      const it = items[i];
      if (!it.movable || it.level !== me.level || it.parent !== me.parent) continue;
      let endKey = it.key;
      if (it.level === 2) {
        for (let j = i + 1; j < items.length && items[j].parent === it.key; j++) endKey = items[j].key;
      }
      const a = rect(it.key);
      const b = rect(endKey);
      if (a && b) out.push({ key: it.key, endKey, top: a.top, bottom: b.bottom });
    }
    return out;
  }

  /** The gap under the pointer among the dragged item's siblings, or null (no target). */
  function targetOf(key: string, x: number, y: number): { gap: number; sibs: Sibling[] } | null {
    const box = root?.getBoundingClientRect();
    if (!box || x < box.left || x >= box.right) return null;
    const sibs = siblings(key);
    const gap = gapAt(sibs, y);
    return gap === null ? null : { gap, sibs };
  }

  function updateTarget(): void {
    if (!dragKey || !lastEvent) return;
    const t = targetOf(dragKey, lastEvent.x, lastEvent.y);
    dropGap = t ? t.gap : null;
    drop = t ? (t.gap < t.sibs.length ? { key: t.sibs[t.gap].key, where: 'before' } : { key: t.sibs[t.sibs.length - 1].endKey, where: 'after' }) : null;
    const logged = t ? String(t.gap) : 'none';
    if (logged !== loggedTarget) {
      loggedTarget = logged;
      void frontendLog(`drag target key=${dragKey} gap=${logged}`);
    }
  }

  function autoScroll(): void {
    frame = 0;
    if (!dragKey) return;
    if (scroller && lastEvent) {
      const box = scroller.getBoundingClientRect();
      if (lastEvent.y < box.top + EDGE) scroller.scrollTop -= SCROLL_STEP;
      else if (lastEvent.y > box.bottom - EDGE) scroller.scrollTop += SCROLL_STEP;
      updateTarget();
    }
    frame = requestAnimationFrame(autoScroll);
  }

  function listen(on: boolean): void {
    const f = on ? window.addEventListener : window.removeEventListener;
    f.call(window, 'pointermove', onMove as EventListener);
    f.call(window, 'pointerup', onUp as EventListener);
    f.call(window, 'pointercancel', onCancel as EventListener);
    f.call(window, 'keydown', onDragKey as EventListener, true);
  }

  function endDrag(): void {
    listen(false);
    if (frame) cancelAnimationFrame(frame);
    frame = 0;
    press = null;
    dragging = false;
    dragKey = null;
    drop = null;
    dropGap = null;
    loggedTarget = null;
    lastEvent = null;
  }

  function onPointerDown(e: PointerEvent, item: Item): void {
    if (!editable || e.button !== 0 || !item.movable || press || reorderAllowed() !== null) return;
    press = { key: item.key, startY: e.clientY };
    lastEvent = { x: e.clientX, y: e.clientY };
    listen(true);
  }

  function onMove(e: PointerEvent): void {
    if (!press) return;
    lastEvent = { x: e.clientX, y: e.clientY };
    if (!dragging) {
      if (Math.abs(e.clientY - press.startY) < DRAG_THRESHOLD) return;
      dragging = true;
      dragKey = press.key;
      void frontendLog(`drag start key=${press.key}`);
      frame = requestAnimationFrame(autoScroll);
    }
    updateTarget();
  }

  function finish(key: string, gap: number | null): void {
    swallowClick = true;
    setTimeout(() => (swallowClick = false), 50);
    endDrag();
    if (gap === null) void frontendLog(`reorder none key=${key} via=drag reason=outside`);
    else void dropTreeItem(key, gap, 'drag').then(() => focusItem(key));
  }

  function onUp(e: PointerEvent): void {
    if (!press) return;
    if (!dragging) return endDrag();
    const key = press.key;
    const t = targetOf(key, e.clientX, e.clientY);
    finish(key, t ? t.gap : null);
  }

  function onCancel(): void {
    if (press && dragging) finish(press.key, null);
    else endDrag();
  }

  function onDragKey(e: KeyboardEvent): void {
    if (e.key !== 'Escape' || !press || !dragging) return;
    e.preventDefault();
    e.stopPropagation();
    finish(press.key, null);
  }

  onDestroy(endDrag);

  const pad = { 1: 'pl-2', 2: 'pl-6', 3: 'pl-10' } as const;
</script>

{#if editable}
  <p id="tree-hint" class="sr-only">Alt+Up and Alt+Down move the focused track or album; Delete deletes it</p>
{/if}
<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
  bind:this={root}
  role="tree"
  aria-label="Tracks"
  tabindex="-1"
  class={cn('flex flex-col', dragKey && 'cursor-grabbing select-none')}
  aria-describedby={editable ? 'tree-hint' : undefined}
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
      data-drop={drop && drop.key === item.key ? drop.where : undefined}
      class={cn(
        'flex cursor-pointer items-center gap-1 rounded-md py-1 pr-2 text-base outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50',
        pad[item.level],
        item.group && 'font-medium',
        item.track && item.track.id === lib.selectedId && 'bg-primary/15 text-foreground shadow-[inset_3px_0_0_var(--primary)]',
        dragKey === item.key && 'opacity-50',
        'data-[drop=before]:shadow-[inset_0_2px_0_var(--primary)] data-[drop=after]:shadow-[inset_0_-2px_0_var(--primary)]',
      )}
      onpointerdown={(e) => onPointerDown(e, item)}
      onclick={() => {
        if (swallowClick) return;
        lib.treeFocus = item.key;
        activate(item);
      }}
      onfocus={() => (lib.treeFocus = item.key)}
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
