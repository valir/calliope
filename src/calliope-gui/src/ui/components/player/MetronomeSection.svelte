<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { pl, setMetronome } from '$lib/player-state.svelte';

  const usable = $derived(pl.status === 'active' && pl.snap?.mode === 'bpm');
  const on = $derived(pl.snap?.metronome.on ?? false);
  const countIn = $derived(pl.snap?.metronome.count_in ?? false);
  const pressed = 'bg-amber-500 text-black hover:bg-amber-500/90';
</script>

<div class="flex flex-col gap-1 rounded-lg border-2 border-border bg-card p-3" role="group" aria-label="Metronome">
  <span class="text-lg font-semibold">Metronome</span>
  <div class="flex items-center gap-2">
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg {on ? pressed : ''}"
      aria-pressed={on}
      disabled={!usable}
      onclick={() => void setMetronome(!on, countIn)}
    >On/Off</Button>
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg {countIn ? pressed : ''}"
      aria-pressed={countIn}
      disabled={!usable}
      onclick={() => void setMetronome(on, !countIn)}
    >Count In</Button>
  </div>
</div>
