<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import ConfirmDialog from '../ConfirmDialog.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Progress } from '$lib/components/ui/progress/index.js';
  import { lib, selectedTrack } from '$lib/library-state.svelte';
  import { frontendLog } from '$lib/ipc';
  import {
    ex, answerConfirm, cancel, exportTo, isRunning, loadInfo,
  } from '$lib/export-state.svelte';
  import {
    confirmText, countInText, disabledReason, exportTarget, progressText, resultText,
  } from '$lib/export-format';

  const target = $derived(exportTarget(lib.treeFocus, selectedTrack()));
  const track = $derived(target.kind === 'track' ? target.track : null);
  const running = $derived(isRunning());
  const reason = $derived(disabledReason(target, ex.info, running));
  const disabled = $derived(reason !== null || ex.preparing);
  const info = $derived(track && ex.info && ex.info.id === track.id ? ex.info : null);
  const resultLine = $derived(ex.message !== '' ? `Export failed: ${ex.message}` : resultText(ex.snap));
  const resultIsError = $derived(ex.message !== '' || ex.snap?.phase === 'failed');

  let confirmOpen = $state(false);
  let confirmMessage = $state('');
  let cancelButton = $state<HTMLElement | null>(null);
  let pane = $state<HTMLElement | null>(null);

  // The export info follows the target track.
  $effect(() => {
    const id = track ? track.id : null;
    untrack(() => void loadInfo(id));
  });

  // One log line per change of target or availability.
  let lastTargetLog = '';
  $effect(() => {
    const id = track ? track.id : 'none';
    const kind = target.kind;
    const available = target.kind === 'track' && info ? info.available : false;
    if (target.kind === 'track' && !info) return;
    const line = `export target id=${id} kind=${kind} available=${available}`;
    if (line === lastTargetLog) return;
    lastTargetLog = line;
    void frontendLog(line);
  });

  $effect(() => {
    if (ex.confirm) {
      confirmMessage = confirmText(ex.confirm.plan.path);
      confirmOpen = true;
    }
  });

  // The focus moves to Cancel when an export starts.
  let wasRunning = false;
  $effect(() => {
    const r = running;
    if (r && !wasRunning) void tick().then(() => cancelButton?.focus());
    wasRunning = r;
  });

  // Logs the pane's overflow and width once its size has settled.
  onMount(() => {
    if (!pane || typeof ResizeObserver === 'undefined') return;
    const el = pane;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ro = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        void frontendLog(`export layout overflow=${el.scrollHeight - el.clientHeight} width=${el.clientWidth}`);
      }, 300);
    });
    ro.observe(el);
    return () => {
      clearTimeout(timer);
      ro.disconnect();
    };
  });

  const viewLook = 'border-transparent bg-transparent dark:bg-transparent';
</script>

<div bind:this={pane} class="flex min-w-0 flex-col gap-4" data-testid="export-pane">
  <div class="grid grid-cols-[7rem_minmax(0,1fr)] items-center gap-x-3 gap-y-1">
    <label for="export-band" class="text-sm font-medium">Band</label>
    <Input id="export-band" value={track?.band ?? ''} readonly disabled={!track} class={viewLook} />
    <label for="export-album" class="text-sm font-medium">Album</label>
    <Input id="export-album" value={track?.album ?? ''} readonly disabled={!track} class={viewLook} />
    <label for="export-title" class="text-sm font-medium">Title</label>
    <Input id="export-title" value={track?.title ?? ''} readonly disabled={!track} class={viewLook} />
  </div>

  <div class="flex flex-col gap-1 text-sm text-muted-foreground">
    <p class="min-h-5" data-testid="export-source">{info?.source ? `Backing track: ${info.source}` : ''}</p>
    <p class="min-h-5" data-testid="export-count-in">{countInText(info)}</p>
  </div>

  <fieldset class="flex flex-col gap-3 rounded-lg border border-border p-4">
    <legend class="px-2 text-sm font-medium">Export...</legend>
    <div class="flex items-center gap-3">
      <Button
        class="h-[3.25rem] w-44"
        {disabled}
        onclick={() => void exportTo('rc5', track)}
      >to BOSS RC-5</Button>
      <label for="export-memory" class="text-sm font-medium">Memory</label>
      <Input
        id="export-memory"
        class="w-16"
        inputmode="numeric"
        autocomplete="off"
        bind:value={ex.memory}
        disabled={running || ex.preparing}
      />
    </div>
    <div class="flex items-center gap-3">
      <Button class="h-[3.25rem] w-44" {disabled} onclick={() => void exportTo('mp3', track)}>MP3 file</Button>
    </div>
    <div class="flex items-center gap-3">
      <Button class="h-[3.25rem] w-44" {disabled} onclick={() => void exportTo('ogg', track)}>OGG file</Button>
    </div>
    <p role="status" class="min-h-5 text-sm text-muted-foreground" data-testid="export-reason">{reason ?? ''}</p>
  </fieldset>

  <div class="flex min-h-[3.5rem] flex-col gap-1" data-testid="export-job">
    {#if running && ex.snap}
      <div class="flex items-center gap-3">
        <Progress value={Math.round(ex.snap.progress * 100)} aria-label="Export progress" class="flex-1" />
        <Button variant="outline" class="h-[3.25rem]" bind:ref={cancelButton} onclick={() => void cancel()}>Cancel</Button>
      </div>
      <p class="text-sm break-all" data-testid="export-progress">{progressText(ex.snap)}</p>
    {:else}
      <p
        role={resultIsError ? 'alert' : 'status'}
        class="min-h-10 text-sm break-all {resultIsError ? 'text-amber-700 dark:text-primary' : ''}"
        data-testid="export-result"
      >{resultLine}</p>
    {/if}
  </div>
</div>

<ConfirmDialog
  bind:open={confirmOpen}
  message={confirmMessage}
  onanswer={(yes) => void answerConfirm(yes)}
/>
