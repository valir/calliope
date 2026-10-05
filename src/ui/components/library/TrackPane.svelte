<script lang="ts">
  import ConfirmDialog from '../ConfirmDialog.svelte';
  import TablaturePanel from './TablaturePanel.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Textarea } from '$lib/components/ui/textarea/index.js';
  import {
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
    if (lib.draft && isDirty(lib.draft)) ask('discard', CONFIRM_DISCARD);
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

  const textFields: { key: DraftField; label: string }[] = [
    { key: 'band', label: 'Band' },
    { key: 'album', label: 'Album' },
    { key: 'title', label: 'Title' },
  ];
  const lateFields: { key: DraftField; label: string }[] = [
    { key: 'source_url', label: 'Source link' },
    { key: 'copyright', label: 'Copyright' },
  ];

  function input(field: DraftField, value: string): void {
    if (lib.draft) lib.draft = setField(lib.draft, field, value);
  }

  const readOnly = $derived([
    { id: 'track-id', label: 'Track ID', value: shown?.id ?? '' },
    { id: 'track-audio', label: 'Audio file', value: shown?.audio ?? '' },
    { id: 'track-imported', label: 'Imported', value: selected?.imported ?? '' },
    { id: 'track-modified', label: 'Modified', value: selected?.modified ?? '' },
  ]);

  // Read-only (view mode) fields drop their box; edit mode shows the full input box.
  const viewLook = $derived(!editing && !noSelection ? 'border-transparent bg-transparent dark:bg-transparent' : '');
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet field(f: { key: DraftField; label: string })}
  <label for="track-{f.key}" class="pt-1.5 text-sm font-medium">{f.label}</label>
  <div class="flex flex-col gap-1">
    <Input
      id="track-{f.key}"
      value={shown ? shown[f.key] : ''}
      disabled={noSelection}
      readonly={!editing}
      class={viewLook}
      aria-invalid={lib.fieldErrors[f.key] ? true : undefined}
      aria-describedby={lib.fieldErrors[f.key] ? `track-${f.key}-error` : undefined}
      oninput={(e) => input(f.key, e.currentTarget.value)}
    />
    {#if lib.fieldErrors[f.key]}
      <p id="track-{f.key}-error" class="text-sm text-destructive">{lib.fieldErrors[f.key]}</p>
    {/if}
  </div>
{/snippet}

<div class="flex h-full min-h-0 flex-col">
  <div class="min-h-0 flex-1 overflow-y-auto pr-1">
    <div class="grid grid-cols-[7rem_minmax(0,1fr)] items-start gap-x-3 gap-y-2">
      {#each textFields as f (f.key)}{@render field(f)}{/each}

      <label for="track-composers" class="pt-1.5 text-sm font-medium">Composers</label>
      <div class="flex flex-col gap-1">
        <Textarea
          id="track-composers"
          rows={3}
          placeholder={editing ? 'One composer per line' : ''}
          value={shown?.composers ?? ''}
          disabled={noSelection}
          readonly={!editing}
          class={viewLook}
          aria-invalid={lib.fieldErrors.composers ? true : undefined}
          aria-describedby={lib.fieldErrors.composers ? 'track-composers-error' : undefined}
          oninput={(e) => input('composers', e.currentTarget.value)}
        />
        {#if lib.fieldErrors.composers}
          <p id="track-composers-error" class="text-sm text-destructive">{lib.fieldErrors.composers}</p>
        {/if}
      </div>

      <label for="track-year" class="pt-1.5 text-sm font-medium">Year</label>
      <div class="flex flex-col gap-1">
        <Input
          id="track-year"
          inputmode="numeric"
          class={`w-28 ${viewLook}`}
          value={shown?.year ?? ''}
          disabled={noSelection}
          readonly={!editing}
          aria-invalid={lib.fieldErrors.year ? true : undefined}
          aria-describedby={lib.fieldErrors.year ? 'track-year-error' : undefined}
          oninput={(e) => input('year', e.currentTarget.value)}
        />
        {#if lib.fieldErrors.year}
          <p id="track-year-error" class="text-sm text-destructive">{lib.fieldErrors.year}</p>
        {/if}
      </div>

      {#each lateFields as f (f.key)}{@render field(f)}{/each}

      {#each readOnly as r (r.id)}
        <label for={r.id} class="pt-1.5 text-sm font-medium text-muted-foreground">{r.label}</label>
        <Input id={r.id} value={r.value} readonly disabled={noSelection} class="text-muted-foreground" />
      {/each}
    </div>

    <div class="mt-4"><TablaturePanel /></div>
  </div>

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
