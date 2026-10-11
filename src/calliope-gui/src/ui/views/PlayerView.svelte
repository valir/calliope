<script lang="ts">
  import { onMount, tick } from 'svelte';
  import TrackBrowser from '../components/library/TrackBrowser.svelte';
  import PlayerPane from '../components/player/PlayerPane.svelte';
  import { load, lib, selectedTrack } from '$lib/library-state.svelte';
  import { attachPlayer, syncSelection, togglePlay, loop, pl } from '$lib/player-state.svelte';
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

  // Elements that keep these keys for themselves (the focus rules of the plan, 2.8).
  const OWN_KEYS =
    'input, textarea, select, button, a[href], [role="checkbox"], [role="slider"], [role="tree"], [role="treeitem"], [role="tab"], [role="combobox"], [role="listbox"], [role="option"], [contenteditable="true"]';

  function plainKey(e: KeyboardEvent): void {
    const t = e.target;
    if (t instanceof Element && t.closest(OWN_KEYS)) return;
    const snap = pl.snap;
    const cur = snap && snap.current_loop !== null ? snap.loops[snap.current_loop] : null;
    if (e.code === 'Space' && !e.shiftKey) {
      e.preventDefault();
      void togglePlay();
    } else if (e.shiftKey) {
      if (e.key !== 'L') return;
      e.preventDefault();
      if (cur && cur.end !== null) void loop({ kind: 'flags', repeat: !cur.repeat, count_in: cur.count_in });
    } else if (e.key === 'b' || e.key === 'e') {
      e.preventDefault();
      void loop({ kind: 'mark', edge: e.key === 'b' ? 'start' : 'end' });
    } else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      void loop({ kind: 'select', delta: e.key === 'ArrowLeft' ? -1 : 1 });
    } else if (e.key === 'l') {
      e.preventDefault();
      if (cur && cur.end !== null) void loop({ kind: 'flags', repeat: !cur.repeat, count_in: cur.count_in });
    }
    // `[` and `]` (speed) come with increment B.
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
      return;
    }
    if (!e.ctrlKey && !e.altKey && !e.metaKey && !e.defaultPrevented) plainKey(e);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<section class="flex h-full min-h-0" aria-labelledby="view-heading">
  <TrackBrowser heading="Player" bind:searchRef {collapsed} onexpand={() => void setCollapsed(false)} />
  <PlayerPane />
</section>
