<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { Progress } from '$lib/components/ui/progress/index.js';
  import TimeField from './TimeField.svelte';
  import { Slider } from '$lib/components/ui/slider/index.js';
  import {
    canPlay, cancelSave, ed, positionText, save, saveEnabled, seek, stop, stopEnabled, togglePlay,
  } from '$lib/editor-state.svelte';

  const active = $derived(ed.status === 'active');
  const soloName = $derived(
    ed.transport.solo ? ed.transport.solo.charAt(0).toUpperCase() + ed.transport.solo.slice(1) : null,
  );
  const saveTarget = $derived(
    ed.variant ? `${ed.variant.name} (${ed.variant.file})` : 'Backing (backings/backing.flac)',
  );

  // The position slider is frozen (shows the drag value) while the user drags it.
  let dragging = $state(false);
  let dragValue = $state(0);
  const sliderValue = $derived(dragging ? dragValue : ed.transport.position_ms);

  function onValue(v: number): void {
    dragValue = v;
    seek(v);
  }
  function endDrag(): void {
    dragging = false;
  }
</script>

<svelte:window onpointerup={endDrag} onpointercancel={endDrag} />

<div class="flex flex-col gap-3 rounded-lg border-2 border-border bg-card p-4" data-testid="mix-lane" role="group" aria-label="Mix">
  <div class="flex flex-wrap items-center gap-4">
    <span class="w-28 shrink-0 text-xl font-semibold">Mix</span>
    <Button class="h-[3.25rem] w-28 text-lg" disabled={!canPlay()} onclick={() => void togglePlay()}>
      {ed.transport.playing ? 'Pause' : 'Play'}
    </Button>
    <Button variant="outline" class="h-[3.25rem] w-24 text-lg" disabled={!stopEnabled()} onclick={() => void stop()}>
      Stop
    </Button>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="min-w-40 flex-1" onpointerdown={() => { dragging = true; dragValue = ed.transport.position_ms; }}>
      <Slider
        value={sliderValue}
        onValueChange={onValue}
        min={0}
        max={Math.max(ed.durationMs, 1)}
        step={100}
        disabled={!active}
        aria-label="Position"
        aria-valuetext={positionText()}
      />
    </div>
    <TimeField />
    {#if soloName}
      <span class="text-lg font-medium text-amber-500" data-testid="solo-text">Solo: {soloName}</span>
    {/if}
    {#if ed.transport.clipping}
      <span class="rounded-md bg-red-600 px-3 py-1 text-lg font-bold text-white" data-testid="clip-badge" role="status">CLIP</span>
    {/if}
  </div>

  <div class="flex flex-wrap items-center gap-4">
    <Button class="h-[3.25rem] w-28 text-lg" disabled={!saveEnabled()} onclick={() => void save()}>Save</Button>
    <span class="text-base text-muted-foreground" data-testid="save-target">Save as: {saveTarget}</span>
    {#if ed.saving !== null}
      <div class="flex min-w-48 flex-1 items-center gap-3" role="status">
        <Progress value={Math.round(ed.saving * 100)} aria-label="Saving progress" />
        <span class="tabular-nums" data-testid="save-percent">{Math.round(ed.saving * 100)}%</span>
        <Button variant="outline" class="h-[3.25rem] px-5 text-lg" onclick={() => void cancelSave()}>Cancel</Button>
      </div>
    {/if}
  </div>

  {#if ed.saveError}
    <p class="text-lg text-destructive" role="alert" data-testid="save-error">{ed.saveError}</p>
  {:else if ed.audioError}
    <p class="text-lg text-destructive" role="alert" data-testid="audio-error">{ed.audioError}</p>
  {/if}
  {#if ed.saveMessage}
    <p class="text-lg {ed.saveMessage.includes('clipped') ? 'text-amber-500' : ''}" role="status" data-testid="save-message">{ed.saveMessage}</p>
  {/if}
</div>
