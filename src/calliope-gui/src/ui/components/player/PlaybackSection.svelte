<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { Slider } from '$lib/components/ui/slider/index.js';
  import TimeField from '../editor/TimeField.svelte';
  import { formatPosition } from '$lib/time-format';
  import { nudge, pl, playLabel, seek, skip, togglePlay } from '$lib/player-state.svelte';

  const active = $derived(pl.status === 'active');
  const duration = $derived(pl.snap?.duration_ms ?? 0);

  // The slider also reports a change when it snaps the playing position to its 100 ms step. Only
  // a change made by the user (a drag, or a key on the slider) is a seek; the slider shows the
  // drag value while the user drags it.
  let dragging = $state(false);
  let keying = $state(false);
  let dragValue = $state(0);
  const sliderValue = $derived(dragging ? dragValue : pl.transport.position_ms);

  function onValue(v: number): void {
    if (!dragging && !keying) return;
    dragValue = v;
    seek(v);
  }
  function endDrag(): void {
    dragging = false;
  }
</script>

<svelte:window onpointerup={endDrag} onpointercancel={endDrag} />

<div class="flex max-w-full grow basis-[23rem] flex-col gap-1 rounded-lg border-2 border-border bg-card p-3" role="group" aria-label="Playback">
  <div class="flex flex-wrap items-center gap-2">
    <Button variant="outline" class="h-[3.25rem] text-lg" disabled={!active} onclick={() => void skip(-5000)}>Rew</Button>
    <Button class="h-[3.25rem] w-24 text-lg" disabled={!active} onclick={() => void togglePlay()}>{playLabel()}</Button>
    <Button variant="outline" class="h-[3.25rem] text-lg" disabled={!active} onclick={() => void skip(5000)}>FF</Button>
    <TimeField
      positionMs={pl.transport.position_ms}
      durationMs={duration}
      {active}
      onseek={seek}
      onnudge={(d) => void nudge(d)}
    />
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    onpointerdowncapture={() => { dragging = true; dragValue = pl.transport.position_ms; }}
    onkeydowncapture={() => { keying = true; }}
    onkeyup={() => { keying = false; }}
    onfocusout={() => { keying = false; }}
  >
    <Slider
      value={sliderValue}
      onValueChange={onValue}
      min={0}
      max={Math.max(duration, 1)}
      step={100}
      disabled={!active}
      aria-label="Position"
      aria-valuetext={formatPosition(pl.transport.position_ms)}
    />
  </div>
</div>
