<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatBpm, parseBpm } from '$lib/bpm-format';
  import { pl, setBpm } from '$lib/player-state.svelte';

  const HINT = 'Comes in a later update';
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

<div class="flex flex-col gap-1 rounded-lg border-2 border-border bg-card p-3" role="group" aria-label="BPM">
  <div class="flex flex-wrap items-center gap-2">
    <label for="bpm-field" class="text-lg font-semibold">BPM</label>
    <input
      id="bpm-field"
      type="text"
      class="w-20 rounded-md border border-input bg-background px-2 py-1 text-center text-2xl font-semibold tabular-nums"
      aria-invalid={invalid !== null ? 'true' : undefined}
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
  </div>
  <div class="flex flex-wrap items-center gap-2">
    <Button variant="outline" class="h-[3.25rem] w-14 text-xl" disabled title={HINT} aria-label="Slower">-</Button>
    <Button variant="outline" class="h-[3.25rem] w-14 text-xl" disabled title={HINT} aria-label="Faster">+</Button>
    <Button variant="outline" class="h-[3.25rem] text-lg" disabled title={HINT}>Reset</Button>
    <Button variant="outline" class="h-[3.25rem] text-lg" disabled title={HINT}>Detect</Button>
  </div>
  <span class="min-h-5 text-sm {invalid ? 'text-destructive' : 'text-muted-foreground'}" role={invalid ? 'alert' : undefined} data-testid="bpm-note">{invalid ?? HINT}</span>
</div>
