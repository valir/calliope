<script lang="ts">
  import { onMount } from 'svelte';
  import ConfirmDialog from '../ConfirmDialog.svelte';
  import TrackFields from '../TrackFields.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { ui } from '$lib/app-state.svelte';
  import { discard, extract, imp, setDraftField } from '$lib/import-state.svelte';
  import { formatLength } from '$lib/format';
  import { getSettings } from '$lib/ipc';

  let keepOriginal = $state<boolean | null>(null);
  let confirmOpen = $state(false);
  let errorEl = $state<HTMLElement | null>(null);

  // At small window sizes the message sits below the fold: scroll it into view when it appears.
  $effect(() => {
    void keepOriginal; // the "original mix" line above appears late and moves the message down
    const el = errorEl;
    if (el) requestAnimationFrame(() => el.scrollIntoView?.({ block: 'nearest' }));
  });

  onMount(async () => {
    try {
      keepOriginal = (await getSettings()).keep_original;
    } catch {
      keepOriginal = null;
    }
  });

  const job = $derived(imp.job);
  const sourceText = $derived(job?.source.label ?? '');
  const lengthText = $derived(formatLength(job?.duration_s ?? null));

  function onKeydown(e: KeyboardEvent): void {
    if (e.key !== 'Escape' || e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
    if (confirmOpen || document.querySelector('[role="alertdialog"]')) return;
    e.preventDefault();
    confirmOpen = true;
  }

  function answer(yes: boolean): void {
    if (yes) void discard();
  }
</script>

<svelte:window onkeydown={onKeydown} />

<section class="flex max-w-2xl flex-col gap-4" aria-label="Track details">
  <h3 class="text-lg font-medium">Track details</h3>

  <TrackFields
    draft={imp.draft}
    editing={true}
    errors={imp.fieldErrors}
    oninput={setDraftField}
    idPrefix="import"
  >
    <span id="import-source-label" class="pt-1.5 text-sm font-medium text-muted-foreground">Source</span>
    <p aria-labelledby="import-source-label" class="break-all pt-1.5 text-sm" data-testid="import-source">{sourceText}</p>
    {#if lengthText}
      <span id="import-length-label" class="pt-1.5 text-sm font-medium text-muted-foreground">Length</span>
      <p aria-labelledby="import-length-label" class="pt-1.5 text-sm">{lengthText}</p>
    {/if}
  </TrackFields>

  {#if keepOriginal !== null}
    <p class="text-sm text-muted-foreground">
      {keepOriginal ? 'The original mix will be kept' : 'The original mix will not be kept'}
      <Button variant="link" size="sm" class="h-auto px-1 text-amber-700 dark:text-primary" onclick={() => {
        ui.settingsTarget = 'stem';
        ui.view = 'settings';
      }}>Change in Settings</Button>
    </p>
  {/if}

  {#if imp.extractError}
    <p bind:this={errorEl} role="alert" class="text-sm text-amber-700 dark:text-primary">{imp.extractError}</p>
  {/if}

  <div class="flex gap-2">
    <Button disabled={imp.starting} onclick={() => void extract()}>Extract</Button>
    <Button variant="outline" disabled={imp.starting} onclick={() => (confirmOpen = true)}>Cancel</Button>
  </div>
</section>

<ConfirmDialog
  bind:open={confirmOpen}
  message="Cancel this import? The prepared audio will be deleted."
  onanswer={answer}
/>
