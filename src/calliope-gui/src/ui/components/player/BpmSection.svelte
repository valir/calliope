<script lang="ts">
  import { formatBpm, parseBpm } from '$lib/bpm-format';
  import { pl, setBpm } from '$lib/player-state.svelte';

  let { nested = false }: { nested?: boolean } = $props();

  const active = $derived(pl.status === 'active');
  // Unsaved text while the user edits; null means the field follows the transport.
  let text = $state<string | null>(null);
  let invalid = $state<string | null>(null);
  const shown = $derived(text ?? formatBpm(pl.transport.bpm));
  const multi = $derived(pl.snap?.tempo.multi ?? false);

  function commit(): void {
    if (text === null) return;
    const v = parseBpm(text);
    if (v === null) {
      invalid = 'Enter a BPM between 20 and 300, or 0 to clear';
      return;
    }
    text = null;
    invalid = null;
    void setBpm(v);
  }
  function revert(): void {
    text = null;
    invalid = null;
  }
  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      revert();
    }
  }
  function onblur(): void {
    if (text !== null && invalid === null) commit();
    if (invalid !== null) revert();
  }
</script>

<!-- Lives in the Sections title row: the song BPM. The speed controls are in the Playback pane. -->
<div class="flex flex-none flex-wrap items-center gap-2 {nested ? '' : 'rounded-lg border-2 border-border bg-card p-3'}" role="group" aria-label="BPM">
  <label for="bpm-field" class="text-lg font-semibold">BPM</label>
  <input
    id="bpm-field"
    type="text"
    class="w-16 rounded-md border border-input bg-background px-1 py-0.5 text-center text-xl font-semibold tabular-nums"
    aria-invalid={invalid !== null ? 'true' : undefined}
    title={invalid ?? undefined}
    disabled={!active}
    value={shown}
    oninput={(e) => { text = e.currentTarget.value; invalid = null; }}
    {onkeydown}
    {onblur}
  />
  <label class="flex items-center gap-1 text-lg" title="The track has several tempos">
    <input type="checkbox" class="size-5" disabled checked={multi} />
    V
  </label>
  {#if invalid}
    <span class="text-sm text-destructive" role="alert" data-testid="bpm-note">{invalid}</span>
  {/if}
</div>
