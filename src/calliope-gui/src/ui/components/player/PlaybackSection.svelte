<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { Slider } from '$lib/components/ui/slider/index.js';
  import TimeField from '../editor/TimeField.svelte';
  import { formatPosition } from '$lib/time-format';
  import { nudge, pl, playLabel, seek, skip, togglePlay } from '$lib/player-state.svelte';

  const active = $derived(pl.status === 'active');
  const duration = $derived(pl.snap?.duration_ms ?? 0);

  function onValue(v: number): void {
    // The slider snaps the incoming position to its 100 ms step and reports that as a change while playing.
    if (v === Math.round(pl.transport.position_ms / 100) * 100) return;
    seek(v);
  }
</script>

<div class="flex min-w-0 flex-1 flex-col gap-1 rounded-lg border-2 border-border bg-card p-3" role="group" aria-label="Playback">
  <div class="flex items-center gap-2">
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
  <Slider
    value={pl.transport.position_ms}
    onValueChange={onValue}
    min={0}
    max={Math.max(duration, 1)}
    step={100}
    disabled={!active}
    aria-label="Position"
    aria-valuetext={formatPosition(pl.transport.position_ms)}
  />
</div>
