<script lang="ts">
  import ConfirmDialog from '../ConfirmDialog.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { exportTablature, frontendLog, pickTablature } from '$lib/ipc';
  import { lib, selectedTrack } from '$lib/library-state.svelte';
  import {
    confirmRemoveTab,
    draftFromRecord,
    stageAdd,
    stageRemove,
    stageUpdate,
    tabButtons,
    tabRows,
    type Result,
  } from '$lib/track-draft';

  let selectedKey = $state<string | null>(null);
  let error = $state('');
  let confirmOpen = $state(false);
  let pendingRemove: { key: string; name: string } | null = null;
  let removeMessage = $state('');
  let busy = $state(false);

  const editing = $derived(lib.mode === 'edit' && lib.draft !== null);
  const shown = $derived.by(() => {
    if (lib.draft) return lib.draft;
    const t = selectedTrack();
    return t ? draftFromRecord(t) : null;
  });
  const rows = $derived(shown ? tabRows(shown) : []);
  const selectedRow = $derived(rows.find((r) => r.key === selectedKey) ?? null);
  const buttons = $derived(tabButtons(editing ? 'edit' : 'view', selectedRow));
  const off = $derived(busy || lib.busy);

  // The selection belongs to one edit session.
  $effect(() => {
    if (!editing) selectedKey = null;
    error = '';
  });
  $effect(() => {
    if (selectedKey !== null && !rows.some((r) => r.key === selectedKey)) selectedKey = null;
  });

  function apply(r: Result, kind: string, pickedName: string, selectNew: boolean): void {
    if (!r.ok) {
      error = r.error;
      return;
    }
    const before = lib.draft?.tabs.map((t) => t.key) ?? [];
    lib.draft = r.draft;
    void frontendLog(`tab-staged ${kind} name=${pickedName}`);
    if (selectNew) {
      const added = r.draft.tabs.find((t) => !before.includes(t.key));
      selectedKey = added?.key ?? selectedKey;
    }
  }

  async function add(): Promise<void> {
    if (!lib.draft) return;
    error = '';
    busy = true;
    try {
      const picked = await pickTablature('add');
      if (!picked || !lib.draft) return;
      const r = stageAdd(lib.draft, picked);
      apply(r, 'add', picked.name, false);
      if (r.ok) {
        // A replace of a removed file reuses its key, so look the row up by name.
        selectedKey = r.draft.tabs.find((t) => t.name === picked.name)?.key ?? null;
      }
    } catch (err) {
      error = `Could not pick a file: ${String(err)}`;
    } finally {
      busy = false;
    }
  }

  async function update(): Promise<void> {
    if (!lib.draft || !selectedRow) return;
    const key = selectedRow.key;
    error = '';
    busy = true;
    try {
      const picked = await pickTablature('update');
      if (!picked || !lib.draft) return;
      apply(stageUpdate(lib.draft, key, picked), 'update', picked.name, false);
    } catch (err) {
      error = `Could not pick a file: ${String(err)}`;
    } finally {
      busy = false;
    }
  }

  function askRemove(): void {
    if (!selectedRow) return;
    pendingRemove = { key: selectedRow.key, name: selectedRow.name };
    removeMessage = confirmRemoveTab(pendingRemove.name);
    confirmOpen = true;
  }

  function answerRemove(yes: boolean): void {
    const p = pendingRemove;
    pendingRemove = null;
    if (!yes || !p || !lib.draft) return;
    error = '';
    apply(stageRemove(lib.draft, p.key), 'remove', p.name, false);
    selectedKey = null;
  }

  async function exportRow(): Promise<void> {
    const t = selectedTrack();
    if (!t || !selectedRow) return;
    error = '';
    busy = true;
    try {
      const path = await exportTablature(t.id, selectedRow.name);
      if (path) lib.message = `Exported to ${path}`;
    } catch (err) {
      error = `Could not export: ${String(err)}`;
    } finally {
      busy = false;
    }
  }

  function onkeydown(e: KeyboardEvent): void {
    if (!editing) return;
    const i = rows.findIndex((r) => r.key === selectedKey);
    let next = i;
    if (e.key === 'ArrowDown') next = Math.min(rows.length - 1, i + 1);
    else if (e.key === 'ArrowUp') next = Math.max(0, i < 0 ? 0 : i - 1);
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = rows.length - 1;
    else return;
    e.preventDefault();
    if (rows[next]) {
      selectedKey = rows[next].key;
      document.getElementById(`tab-opt-${selectedKey}`)?.scrollIntoView?.({ block: 'nearest' });
    }
  }
</script>

<section aria-labelledby="tab-files-label" class="flex flex-col gap-2">
  <h2 id="tab-files-label" class="text-sm font-medium">Tablature Files</h2>
  <div class="rounded-lg border border-input" class:opacity-60={!editing}>
    <ul
      role="listbox"
      tabindex={editing ? 0 : -1}
      aria-labelledby="tab-files-label"
      aria-disabled={!editing}
      aria-activedescendant={selectedKey ? `tab-opt-${selectedKey}` : undefined}
      class="max-h-32 min-h-8 overflow-y-auto outline-none focus-visible:ring-3 focus-visible:ring-ring/50"
      {onkeydown}
    >
      {#each rows as row (row.key)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id="tab-opt-{row.key}"
          role="option"
          aria-selected={row.key === selectedKey}
          class="flex h-8 cursor-default items-center justify-between gap-2 px-2.5 text-sm aria-selected:bg-primary/15 aria-selected:shadow-[inset_3px_0_0_var(--primary)]"
          onclick={() => editing && (selectedKey = row.key)}
        >
          <span class="truncate">{row.name}</span>
          {#if row.state !== 'saved'}
            <span class="shrink-0 text-xs font-medium text-amber-700 dark:text-primary">{row.state}</span>
          {/if}
        </li>
      {/each}
    </ul>
  </div>
  {#if error}
    <p role="alert" class="text-sm text-destructive">{error}</p>
  {/if}
  <div class="flex flex-wrap gap-2">
    <Button variant="outline" size="sm" disabled={off || !buttons.add} onclick={add}>Add</Button>
    <Button variant="outline" size="sm" disabled={off || !buttons.update} onclick={update}>Update</Button>
    <Button variant="outline" size="sm" disabled={off || !buttons.export} onclick={exportRow}>Export</Button>
    <Button variant="outline" size="sm" disabled={off || !buttons.remove} onclick={askRemove}>Remove</Button>
  </div>
</section>

<ConfirmDialog
  bind:open={confirmOpen}
  message={removeMessage}
  onanswer={answerRemove}
/>
