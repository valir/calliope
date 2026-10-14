<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { parseBpm } from '$lib/bpm-format';

  interface Props {
    /** Left edge in px inside the waveform. */
    xPx: number;
    initial: number;
    /** Accessible name. */
    label: string;
    oncommit: (bpm: number) => void;
    oncancel: () => void;
    oninvalid: (message: string) => void;
  }
  let { xPx, initial, label, oncommit, oncancel, oninvalid }: Props = $props();

  let input: HTMLInputElement | undefined = $state();
  let value = $state('');
  let done = false;

  onMount(() => {
    value = `${Number(initial.toFixed(2))}`;
    input?.focus();
    void tick().then(() => input?.select());
  });

  function onkeydown(e: KeyboardEvent): void {
    e.stopPropagation();
    if (e.key === 'Enter') {
      e.preventDefault();
      const bpm = parseBpm(value);
      if (bpm === null || bpm === 0) {
        oninvalid('Enter a BPM between 20 and 300');
        return;
      }
      done = true;
      oncommit(bpm);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      done = true;
      oncancel();
    }
  }

  function onblur(): void {
    if (done) return;
    done = true;
    oncancel();
  }
</script>

<input
  bind:this={input}
  bind:value
  type="text"
  inputmode="decimal"
  aria-label={label}
  data-bpm-input
  data-testid="section-bpm-input"
  class="absolute bottom-1 h-9 w-24 rounded border border-border bg-background px-2 text-base text-foreground"
  style:left="{xPx}px"
  {onkeydown}
  {onblur}
/>
