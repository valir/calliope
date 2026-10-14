<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { Progress } from '$lib/components/ui/progress/index.js';
  import TimeField from './TimeField.svelte';
  import { Slider } from '$lib/components/ui/slider/index.js';
  import WaveformView from '../player/WaveformView.svelte';
  import {
    canDetect, canPlay, cancelDetect, cancelSave, detect, detectHint, ed, moveChange, nudge, positionText, save, saveEnabled, seek, stop, stopEnabled, togglePlay,
  } from '$lib/editor-state.svelte';

  const active = $derived(ed.status === 'active');
  const soloName = $derived(
    ed.transport.solo ? ed.transport.solo.charAt(0).toUpperCase() + ed.transport.solo.slice(1) : null,
  );
  const saveTarget = $derived(
    ed.variant ? `${ed.variant.name} (${ed.variant.file})` : 'Backing (backings/backing.flac)',
  );

  const PHASE_TEXT: Record<string, string> = {
    checking: 'Checking the edge-AI server',
    mixing: 'Preparing the audio',
    uploading: 'Uploading',
    waiting: 'Waiting for the edge-AI server',
    detecting: 'Detecting beats',
    applying: 'Saving the tempo',
  };
  const phaseText = $derived.by(() => {
    const d = ed.detect;
    if (!d) return '';
    const base = PHASE_TEXT[d.phase] ?? d.phase;
    return d.progress == null ? base : `${base} ${Math.round(d.progress * 100)}%`;
  });
  const hint = $derived(detectHint());

  // The position slider is frozen (shows the drag value) while the user drags it.
  let dragging = $state(false);
  let dragValue = $state(0);
  const sliderValue = $derived(dragging ? dragValue : ed.transport.position_ms);

  function onValue(v: number): void {
    // While playing, the slider snaps the incoming position to its 100 ms step and reports that
    // as a change; it is not the user moving it (it would seek on every transport event).
    if (!dragging && v === Math.round(ed.transport.position_ms / 100) * 100) return;
    dragValue = v;
    seek(v);
  }
  function endDrag(): void {
    dragging = false;
  }
</script>

<svelte:window onpointerup={endDrag} onpointercancel={endDrag} />

<div class="flex shrink-0 flex-col gap-1.5 rounded-lg border-2 border-border bg-card p-2" data-testid="mix-lane" role="group" aria-label="Mix">
  <WaveformView
    testid="mix-waveform"
    label="Mix waveform"
    peaks={ed.peaks}
    durationMs={ed.durationMs}
    positionMs={ed.transport.position_ms}
    segments={ed.tempo.segments}
    onmove={moveChange}
    locked={ed.detect !== null}
    class="h-14"
  />

  <div class="flex flex-wrap items-center gap-x-3 gap-y-1.5">
    <!-- Fixed-size cell: the solo name and the CLIP badge appear here without moving anything. -->
    <div class="flex h-[3.25rem] w-28 shrink-0 flex-col justify-between">
      <div class="flex items-center gap-2">
        <span class="text-xl font-semibold">Mix</span>
        {#if ed.transport.clipping}
          <span class="rounded-md bg-red-600 px-2 text-base font-bold leading-6 text-white" data-testid="clip-badge" role="status">CLIP</span>
        {/if}
      </div>
      <span class="h-6 truncate text-base font-medium leading-6 text-amber-700 dark:text-amber-500" data-testid="solo-text">{#if soloName}Solo: {soloName}{/if}</span>
    </div>
    <Button class="h-[3.25rem] w-24 text-lg" disabled={!canPlay()} onclick={() => void togglePlay()}>
      {ed.transport.playing ? 'Pause' : 'Play'}
    </Button>
    <Button variant="outline" class="h-[3.25rem] w-20 text-lg" disabled={!stopEnabled()} onclick={() => void stop()}>
      Stop
    </Button>
    <span class="flex-1"></span>
    {#if ed.detect !== null}
      <Button variant="outline" class="h-[3.25rem] w-44 shrink-0 text-lg" onclick={() => void cancelDetect()}>Cancel</Button>
    {:else}
      <Button variant="outline" class="h-[3.25rem] w-44 shrink-0 text-lg" disabled={!canDetect()} title={hint} aria-describedby={hint ? 'detect-hint' : undefined} onclick={() => void detect()}>Autodetect BPM</Button>
    {/if}
  </div>

  <div class="flex items-center gap-3">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="min-w-0 flex-1" onpointerdown={() => { dragging = true; dragValue = ed.transport.position_ms; }}>
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
    <TimeField
      positionMs={ed.transport.position_ms}
      durationMs={ed.durationMs}
      {active}
      onseek={seek}
      onnudge={(d) => void nudge(d)}
    />
  </div>

  <div class="flex items-center gap-3">
    <Button class="h-[3.25rem] w-28 shrink-0 text-lg" disabled={!saveEnabled()} onclick={() => void save()}>Save</Button>
    {#if ed.saving !== null}
      <div class="flex min-w-0 flex-1 items-center gap-3" role="status">
        <Progress value={Math.round(ed.saving * 100)} aria-label="Saving progress" />
        <span class="w-12 shrink-0 text-right tabular-nums" data-testid="save-percent">{Math.round(ed.saving * 100)}%</span>
        <Button variant="outline" class="h-[3.25rem] shrink-0 px-5 text-lg" onclick={() => void cancelSave()}>Cancel</Button>
      </div>
    {:else}
      <span class="min-w-0 flex-1 truncate text-base text-muted-foreground" title={saveTarget} data-testid="save-target">Save as: {saveTarget}</span>
    {/if}
  </div>

  <!-- One reserved slot (two lines of text-base) for errors and the save result, so the lane never grows. -->
  <div class="min-h-12">
    {#if ed.saveError}
      <p class="text-base text-destructive" role="alert" data-testid="save-error">{ed.saveError}</p>
    {:else if ed.audioError}
      <p class="text-base text-destructive" role="alert" data-testid="audio-error">{ed.audioError}</p>
    {/if}
    {#if ed.detectError}
      <p class="text-base text-destructive" role="alert" data-testid="detect-error">{ed.detectError}</p>
    {:else if ed.detectMessage}
      <p class="text-base" role="status" data-testid="detect-message">{ed.detectMessage}</p>
    {/if}
    {#if phaseText}
      <p class="text-base" role="status" data-testid="detect-status">{phaseText}</p>
    {:else if hint && !ed.detectError && !ed.detectMessage && !ed.saveMessage && !ed.saveError && !ed.audioError}
      <p class="text-base text-muted-foreground" id="detect-hint" data-testid="detect-hint">{hint}</p>
    {/if}
    {#if ed.saveMessage}
      <p class="text-base {ed.saveMessage.includes('clipped') ? 'text-amber-700 dark:text-amber-500' : ''}" role="status" data-testid="save-message">{ed.saveMessage}</p>
    {/if}
  </div>
</div>
