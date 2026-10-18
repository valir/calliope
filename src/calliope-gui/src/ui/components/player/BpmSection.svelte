<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatBpm, parseBpm } from '$lib/bpm-format';
  import { pl, setBpm } from '$lib/player-state.svelte';

  let { nested = false }: { nested?: boolean } = $props();

  const HINT = 'Comes in a later update';
  const SHOW_PLACEHOLDERS = false;
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

<!-- Lives in the Sections title row. The -/+/Reset placeholders and the "later update" hint stay hidden until
     that feature exists: set SHOW_PLACEHOLDERS to true to bring them back. -->
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
  {#if SHOW_PLACEHOLDERS}
    <Button variant="outline" class="h-[3.25rem] w-14 text-xl" disabled title={HINT} aria-label="Slower">-</Button>
    <Button variant="outline" class="h-[3.25rem] w-14 text-xl" disabled title={HINT} aria-label="Faster">+</Button>
    <Button variant="outline" class="h-[3.25rem] text-lg" disabled title={HINT}>Reset</Button>
  {/if}
  {#if invalid || SHOW_PLACEHOLDERS}
    <span class="text-sm {invalid ? 'text-destructive' : 'text-muted-foreground'}" role={invalid ? 'alert' : undefined} data-testid="bpm-note">{invalid ?? HINT}</span>
  {/if}
</div>
