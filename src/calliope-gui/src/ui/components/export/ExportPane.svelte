<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import ConfirmDialog from '../ConfirmDialog.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import { Switch } from '$lib/components/ui/switch/index.js';
  import { Progress } from '$lib/components/ui/progress/index.js';
  import * as RadioGroup from '$lib/components/ui/radio-group/index.js';
  import { lib, selectedTrack } from '$lib/library-state.svelte';
  import { frontendLog } from '$lib/ipc';
  import {
    ex, answerConfirm, cancel, changeRc5Folder, changeRc500Folder, exportTo, isRunning, loadInfo, setSlowCountIn,
  } from '$lib/export-state.svelte';
  import {
    confirmText, countInText, disabledReason, exportTarget, lastText, progressText, resultText,
  } from '$lib/export-format';

  const target = $derived(exportTarget(lib.treeFocus, selectedTrack()));
  const track = $derived(target.kind === 'track' ? target.track : null);
  const running = $derived(isRunning());
  const reason = $derived(disabledReason(target, ex.info, running));
  const disabled = $derived(reason !== null || ex.preparing || ex.slowSaving);
  const info = $derived(track && ex.info && ex.info.id === track.id ? ex.info : null);
  const slowDisabled = $derived(
    !info?.count_in || !info.available || running || ex.preparing || ex.confirm !== null || ex.slowSaving,
  );
  const resultLine = $derived(
    ex.message !== '' ? `Export failed: ${ex.message}` : ex.notice !== '' ? ex.notice : resultText(ex.snap),
  );
  const resultIsError = $derived(ex.message !== '' || ex.notice !== '' || ex.snap?.phase === 'failed');

  let confirmOpen = $state(false);
  let confirmMessage = $state('');
  let cancelButton = $state<HTMLElement | null>(null);
  let pane = $state<HTMLElement | null>(null);

  // The export info follows the target track.
  const targetId = $derived(track ? track.id : null);
  $effect(() => {
    const id = targetId;
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

  // One log line per change of the count-in text or of the track (GUI tests read it).
  let lastCountInLog = '';
  $effect(() => {
    if (!info?.count_in) return;
    const line = `export count-in id=${info.id} slow=${info.count_in.slow} text=${countInText(info)}`;
    if (line === lastCountInLog) return;
    lastCountInLog = line;
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
    // The pane grows with its content; the scrolling happens in the section around it.
    const el = pane.parentElement ?? pane;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ro = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        void frontendLog(
          `export layout overflow=${el.scrollHeight - el.clientHeight} width=${el.clientWidth} ` +
            `hscroll=${el.scrollWidth - el.clientWidth}`,
        );
        void frontendLog(geometryLine());
      }, 300);
    });
    ro.observe(el);
    ro.observe(pane);
    return () => {
      clearTimeout(timer);
      ro.disconnect();
    };
  });

  /** The centres of the controls in window coordinates (the GUI tests take their click points from it). */
  function geometryLine(): string {
    const at = (sel: string): string => {
      const e = pane?.querySelector(sel);
      if (!e) return 'none';
      const b = e.getBoundingClientRect();
      const k = window.devicePixelRatio || 1;
      return `${Math.round((b.left + b.width / 2) * k)},${Math.round((b.top + b.height / 2) * k)}`;
    };
    return (
      `export geometry rc5=${at('[data-testid="export-rc5-button"]')} memory=${at('#export-memory')} ` +
      `rc500=${at('[data-testid="export-rc500-button"]')} rc500_memory=${at('#export-rc500-memory')} ` +
      `track1=${at('#export-rc500-track-1')} track2=${at('#export-rc500-track-2')} ` +
      `mp3=${at('[data-testid="export-mp3-button"]')} ogg=${at('[data-testid="export-ogg-button"]')} ` +
      `slow=${at('#export-slow-count-in')}`
    );
  }

  const viewLook = 'border-transparent bg-transparent dark:bg-transparent';
</script>

<div bind:this={pane} class="flex min-w-0 flex-col gap-3" data-testid="export-pane">
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
    <div class="flex min-h-6 items-center gap-3">
      <Switch
        id="export-slow-count-in"
        data-testid="export-slow-count-in"
        bind:checked={() => info?.count_in?.slow ?? false, (v) => void setSlowCountIn(v)}
        disabled={slowDisabled}
      />
      <Label for="export-slow-count-in">Slow count-in (half tempo)</Label>
    </div>
  </div>

  <fieldset class="@container flex min-w-0 flex-col gap-2 rounded-lg border border-border p-4 pb-3">
    <legend class="px-2 text-sm font-medium">Export...</legend>
    <!-- Two columns when there is room (RC-5 | RC-500, MP3 | OGG), one otherwise. Each "Last:" line sits under its
         button, so the heights do not depend on the text. -->
    <div class="grid grid-cols-1 gap-x-4 gap-y-2 @min-[576px]:grid-cols-2">
      <div class="flex min-w-0 flex-col gap-1" role="group" aria-label="BOSS RC-5" data-testid="export-rc5">
        <div class="flex min-h-11 items-center gap-3">
          <Button
            class="h-11 w-40"
            data-testid="export-rc5-button"
            {disabled}
            onclick={() => void exportTo('rc5', track)}
          >to BOSS RC-5</Button>
        </div>
        <div class="flex min-h-9 items-center gap-3">
          <label for="export-memory" class="text-sm font-medium">Memory</label>
          <Input
            id="export-memory"
            class="w-16"
            inputmode="numeric"
            autocomplete="off"
            bind:value={ex.memory}
            disabled={running || ex.preparing}
          />
          {#if ex.rc5Quick}
            <Button
              variant="outline"
              size="sm"
              class="shrink-0"
              data-testid="export-rc5-change"
              disabled={running || ex.preparing}
              onclick={() => void changeRc5Folder()}
            >Change...</Button>
          {/if}
        </div>
        <!-- Reserved row: it keeps its height with or without a folder, so nothing below it moves. -->
        <div class="min-h-10 min-w-0" data-testid="export-rc5-to">
          {#if ex.rc5Quick}
            <p class="min-w-0 break-all text-sm" data-testid="export-rc5-folder">To: {ex.rc5Quick}</p>
          {/if}
        </div>
        <p class="min-h-5 min-w-0 break-all text-sm text-muted-foreground" data-testid="export-last-rc5">{lastText('rc5', info?.last)}</p>
      </div>
      <div class="flex min-w-0 flex-col gap-1" role="group" aria-label="BOSS RC-500" data-testid="export-rc500">
        <div class="flex min-h-11 items-center gap-3">
          <Button
            class="h-11 w-40"
            data-testid="export-rc500-button"
            {disabled}
            onclick={() => void exportTo('rc500', track)}
          >to BOSS RC-500</Button>
          <RadioGroup.Root
            value={String(ex.rc500Track)}
            onValueChange={(v) => (ex.rc500Track = v === '2' ? 2 : 1)}
            disabled={running || ex.preparing}
            aria-label="RC-500 track"
            class="gap-1"
          >
            {#each [1, 2] as t (t)}
              <div class="flex items-center gap-2">
                <RadioGroup.Item value={String(t)} id="export-rc500-track-{t}" />
                <Label for="export-rc500-track-{t}" class="text-sm">Track {t}</Label>
              </div>
            {/each}
          </RadioGroup.Root>
        </div>
        <div class="flex min-h-9 items-center gap-3">
          <span class="text-sm font-medium" aria-hidden="true">Memory</span>
          <Input
            id="export-rc500-memory"
            aria-label="RC-500 memory"
            class="w-16"
            inputmode="numeric"
            autocomplete="off"
            bind:value={ex.rc500Memory}
            disabled={running || ex.preparing}
          />
          {#if ex.rc500Quick}
            <Button
              variant="outline"
              size="sm"
              class="shrink-0"
              data-testid="export-rc500-change"
              disabled={running || ex.preparing}
              onclick={() => void changeRc500Folder()}
            >Change...</Button>
          {/if}
        </div>
        <div class="min-h-10 min-w-0" data-testid="export-rc500-to">
          {#if ex.rc500Quick}
            <p class="min-w-0 break-all text-sm" data-testid="export-rc500-folder">To: {ex.rc500Quick}</p>
          {/if}
        </div>
        <p class="min-h-5 min-w-0 break-all text-sm text-muted-foreground" data-testid="export-last-rc500">{lastText('rc500', info?.last)}</p>
      </div>
      <div class="flex min-w-0 flex-col gap-1">
        <Button class="h-11 w-40" data-testid="export-mp3-button" {disabled} onclick={() => void exportTo('mp3', track)}>MP3 file</Button>
        <p class="min-h-10 min-w-0 break-all text-sm text-muted-foreground" data-testid="export-last-mp3">{lastText('mp3', info?.last)}</p>
      </div>
      <div class="flex min-w-0 flex-col gap-1">
        <Button class="h-11 w-40" data-testid="export-ogg-button" {disabled} onclick={() => void exportTo('ogg', track)}>OGG file</Button>
        <p class="min-h-10 min-w-0 break-all text-sm text-muted-foreground" data-testid="export-last-ogg">{lastText('ogg', info?.last)}</p>
      </div>
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
      {#if ex.snap?.phase === 'done' && ex.snap.info_error}
        <p role="alert" class="text-sm break-all text-amber-700 dark:text-primary" data-testid="export-info-error">{ex.snap.info_error}</p>
      {/if}
    {/if}
  </div>
</div>

<ConfirmDialog
  bind:open={confirmOpen}
  message={confirmMessage}
  onanswer={(yes) => void answerConfirm(yes)}
/>
