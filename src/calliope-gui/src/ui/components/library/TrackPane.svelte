<script lang="ts">
  import ConfirmDialog from '../ConfirmDialog.svelte';
  import TablaturePanel from './TablaturePanel.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import TrackFields from '../TrackFields.svelte';
  import {
    discardAfterConflict,
    endEdit,
    exportSelectedTrack,
    lib,
    removeSelectedTrack,
    saveEdit,
    selectedTrack,
    startEdit,
  } from '$lib/library-state.svelte';
  import {
    CONFIRM_DISCARD,
    confirmDeleteTrack,
    draftFromRecord,
    isDirty,
    paneButtons,
    setField,
    type DraftField,
  } from '$lib/track-draft';

  const selected = $derived(selectedTrack());
  const shown = $derived(lib.draft ?? (selected ? draftFromRecord(selected) : null));
  const editing = $derived(lib.mode === 'edit' && lib.draft !== null);
  const buttons = $derived(paneButtons(lib.mode, selected !== null, lib.busy));
  const noSelection = $derived(shown === null);

  let confirmOpen = $state(false);
  let confirmMessage = $state('');
  let pending: 'discard' | 'delete' | null = null;

  function ask(kind: 'discard' | 'delete', message: string): void {
    pending = kind;
    confirmMessage = message;
    confirmOpen = true;
  }

  function cancel(): void {
    if (!editing) return;
    if (lib.conflict) void discardAfterConflict();
    else if (lib.draft && isDirty(lib.draft)) ask('discard', CONFIRM_DISCARD);
    else endEdit();
  }

  function askDelete(): void {
    if (selected) ask('delete', confirmDeleteTrack(selected.title));
  }

  function answer(yes: boolean): void {
    const p = pending;
    pending = null;
    if (!yes) return;
    if (p === 'discard') endEdit();
    else if (p === 'delete') void removeSelectedTrack();
  }

  function onKeydown(e: KeyboardEvent): void {
    if (confirmOpen || document.querySelector('[role="alertdialog"]')) return;
    const plain = !e.altKey && !e.shiftKey && !e.metaKey;
    if (e.ctrlKey && plain && e.code === 'KeyE') {
      e.preventDefault();
      if (buttons.edit) startEdit();
    } else if (e.ctrlKey && plain && e.code === 'KeyS') {
      e.preventDefault();
      if (buttons.save) void saveEdit();
    } else if (e.key === 'Escape' && !e.ctrlKey && plain && buttons.cancel) {
      e.preventDefault();
      cancel();
    }
  }

  function input(field: DraftField, value: string): void {
    if (lib.draft) lib.draft = setField(lib.draft, field, value);
  }

  const readOnly = $derived([
    { id: 'track-id', label: 'Track ID', value: shown?.id ?? '' },
    { id: 'track-type', label: 'Type', value: selected ? (selected.type === 'stem' ? 'Stem track' : 'Backing track') : '' },
    { id: 'track-audio', label: 'Audio file', value: selected ? (selected.audio ?? 'none') : '' },
    ...(selected?.original ? [{ id: 'track-original', label: 'Original', value: selected.original }] : []),
    ...(selected?.stem_model ? [{ id: 'track-stem-model', label: 'Stem model', value: selected.stem_model }] : []),
    { id: 'track-imported', label: 'Imported', value: selected?.imported ?? '' },
    { id: 'track-modified', label: 'Modified', value: selected?.modified ?? '' },
  ]);
</script>

<svelte:window onkeydown={onKeydown} />

<div class="flex h-full min-h-0 flex-col">
  <div class="min-h-0 flex-1 overflow-y-auto pr-1">
    <TrackFields
      draft={shown}
      {editing}
      errors={lib.fieldErrors}
      oninput={input}
      idPrefix="track"
    >
      {#each readOnly as r (r.id)}
        <label for={r.id} class="pt-1.5 text-sm font-medium text-muted-foreground">{r.label}</label>
        <Input id={r.id} value={r.value} readonly disabled={noSelection} class="text-muted-foreground" />
      {/each}
      {#if selected && selected.stems.length > 0}
        <span id="track-stems-label" class="pt-1.5 text-sm font-medium text-muted-foreground">Stems</span>
        <ul aria-labelledby="track-stems-label" class="flex flex-wrap gap-1.5 pt-1">
          {#each selected.stems as st (st.name)}
            <li
              class="rounded-md border border-border px-2 py-0.5 text-sm text-muted-foreground"
              class:text-destructive={selected.missing.includes(st.file)}
            >
              {st.name}
            </li>
          {/each}
        </ul>
      {/if}
      {#if selected && selected.backings.length > 0}
        <span id="track-backings-label" class="pt-1.5 text-sm font-medium text-muted-foreground">Backing tracks</span>
        <ul aria-labelledby="track-backings-label" class="flex flex-wrap gap-1.5 pt-1" data-testid="track-backings">
          {#each selected.backings as b (b.id)}
            <li class="rounded-md border border-border px-2 py-0.5 text-sm text-muted-foreground">{b.name} ({b.file})</li>
          {/each}
        </ul>
      {/if}
    </TrackFields>
  </div>

  <!-- The tablature panel and the action bar are pinned below the scrolling fields, so their
       buttons are always visible. -->
  <div class="shrink-0 border-t border-border pt-3"><TablaturePanel /></div>

  <div class="shrink-0 pt-3">
    {#if lib.errorMessage}
      <p role="alert" class="mb-2 text-sm text-destructive">{lib.errorMessage}</p>
    {:else if lib.message}
      <p role="status" class="mb-2 break-all text-sm text-muted-foreground">{lib.message}</p>
    {/if}
    <div class="flex flex-wrap gap-2 border-t border-border pt-3" role="toolbar" aria-label="Track actions">
      <Button size="sm" disabled={!buttons.edit} onclick={startEdit}>Edit</Button>
      <Button size="sm" disabled={!buttons.save} onclick={() => void saveEdit()}>Save</Button>
      <Button variant="outline" size="sm" disabled={!buttons.cancel} onclick={cancel}>Cancel</Button>
      <Button variant="outline" size="sm" disabled={!buttons.export} onclick={() => void exportSelectedTrack()}>Export</Button>
      <Button variant="destructive" size="sm" disabled={!buttons.delete} onclick={askDelete}>Delete</Button>
    </div>
  </div>
</div>

<ConfirmDialog bind:open={confirmOpen} message={confirmMessage} onanswer={answer} />
