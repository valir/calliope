<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { MAX_SPEED, MIN_SPEED, readoutText } from '$lib/speed-format';
  import { clearSpeedError, pl, shownSpeed, speedLocked, speedReset, speedSet, speedStep } from '$lib/player-state.svelte';

  const active = $derived(pl.status === 'active');
  const locked = $derived(speedLocked());
  const speed = $derived(shownSpeed());
  const RUNNING = 'Speed training is running; stop the loop to change the speed';
  const NOT_ACTIVE = 'Open a track first';
  const off = $derived(!active || locked);
  const why = $derived(!active ? NOT_ACTIVE : locked ? RUNNING : undefined);
  const noSlower = $derived(off || speed <= MIN_SPEED);
  const noFaster = $derived(off || speed >= MAX_SPEED);
  const noReset = $derived(off || speed === 100);
  const readout = $derived(readoutText(pl.transport.played_bpm));

  // Unsaved text while the user edits; null means the box follows the speed.
  let text = $state<string | null>(null);
  const shown = $derived(text ?? String(speed));
  const invalid = $derived(pl.speedError !== '');

  async function commit(): Promise<void> {
    if (text === null) return;
    const t = text;
    await speedSet(t);
    if (pl.speedError === '') text = null;
  }
  function revert(): void {
    text = null;
    clearSpeedError();
  }
  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      void commit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      revert();
    }
  }
  function onblur(): void {
    if (text === null) return;
    if (invalid) revert();
    else void commit();
  }
</script>

<div class="flex flex-none items-center gap-2" role="group" aria-label="Speed">
  <span class="text-lg font-semibold">Speed</span>
  <Button variant="outline" class="h-[3.25rem] w-14 text-xl" aria-label="Slower" disabled={noSlower}
    title={why ?? (noSlower ? `The slowest speed is ${MIN_SPEED} %` : undefined)}
    onclick={() => void speedStep(-1, 'minus')}>-</Button>
  <input
    type="text"
    inputmode="numeric"
    class="w-16 rounded-md border border-input bg-background px-1 py-0.5 text-center text-xl font-semibold tabular-nums disabled:opacity-50 aria-invalid:border-destructive"
    aria-label="Speed in percent"
    aria-invalid={invalid ? 'true' : undefined}
    title={invalid ? pl.speedError : why}
    disabled={off}
    value={shown}
    oninput={(e) => { text = e.currentTarget.value; clearSpeedError(); }}
    {onkeydown}
    {onblur}
  />
  <span class="text-lg">%</span>
  <Button variant="outline" class="h-[3.25rem] w-14 text-xl" aria-label="Faster" disabled={noFaster}
    title={why ?? (noFaster ? `The fastest speed is ${MAX_SPEED} %` : undefined)}
    onclick={() => void speedStep(1, 'plus')}>+</Button>
  <Button variant="outline" class="h-[3.25rem] text-lg" disabled={noReset}
    title={why ?? (noReset ? 'Already at 100 %' : undefined)}
    onclick={() => void speedReset()}>Reset</Button>
  <span
    class="min-w-[6.5rem] text-xl font-semibold tabular-nums {invalid ? 'text-destructive' : ''}"
    aria-label={invalid ? undefined : 'Playing BPM'}
    data-testid="bpm-readout"
  >{invalid ? `${MIN_SPEED}–${MAX_SPEED} %` : readout}</span>
</div>
