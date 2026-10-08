<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { Checkbox } from '$lib/components/ui/checkbox/index.js';
  import { Slider } from '$lib/components/ui/slider/index.js';
  import { ed, lanePlay, resetGain, setGain, setUnmuted } from '$lib/editor-state.svelte';
  import { formatGain, gainToSlider, sliderToGain } from '$lib/gain-format';
  import type { LaneState } from '$lib/ipc';

  let { lane }: { lane: LaneState } = $props();

  const label = $derived(lane.name.charAt(0).toUpperCase() + lane.name.slice(1));
  const soloed = $derived(ed.transport.solo === lane.name);
  const dimmed = $derived(ed.transport.solo !== null && !soloed);
  const text = $derived(formatGain(lane.gain_db));
</script>

<div
  class="flex items-center gap-3 rounded-lg border border-border px-3 py-2 {dimmed ? 'opacity-50' : ''}"
  data-testid="lane-{lane.name}"
  data-dimmed={dimmed}
  role="group"
  aria-label={label}
>
  <span class="w-24 shrink-0 text-xl font-medium">{label}</span>
  <Button
    variant={soloed ? 'default' : 'outline'}
    class="h-[3.25rem] w-24 shrink-0 text-lg {soloed ? 'bg-amber-500 text-black hover:bg-amber-400' : ''}"
    aria-pressed={soloed}
    aria-label="{soloed ? 'Solo' : 'Play'} {label}"
    onclick={() => void lanePlay(lane.name)}
  >
    {soloed ? 'Solo' : 'Play'}
  </Button>
  <label class="flex shrink-0 items-center gap-2 text-lg">
    <Checkbox
      checked={lane.unmuted}
      onCheckedChange={(c) => setUnmuted(lane.name, c === true)}
      aria-label="Unmute {label}"
    />
    Unmute
  </label>
  <div class="min-w-24 flex-1">
    <Slider
      value={gainToSlider(lane.gain_db)}
      onValueChange={(v) => setGain(lane.name, sliderToGain(v))}
      min={-60}
      max={12}
      step={0.5}
      tick={0}
      aria-label="Volume {label}"
      aria-valuetext={text}
    />
  </div>
  <Button
    variant="ghost"
    class="h-[3.25rem] w-[6.5rem] shrink-0 px-1 text-lg tabular-nums"
    aria-label="{label} volume {text}, reset to 0 dB"
    title="Reset to 0 dB"
    data-testid="gain-text-{lane.name}"
    onclick={() => resetGain(lane.name)}
  >
    {text}
  </Button>
</div>
