<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatBpm, scaleBpm } from '$lib/bpm-format';
  import { pl, removeChange, scaleSection, startAdd, targetSection } from '$lib/player-state.svelte';

  const segs = $derived(pl.snap?.tempo.segments ?? []);
  const none = $derived(pl.status !== 'active' || segs.length === 0);
  const target = $derived(none ? null : targetSection());
  const bpm = $derived(target === null ? null : segs[target].bpm);
  const status = $derived(
    target === null || bpm === null ? 'No tempo' : `Section ${target + 1} of ${segs.length} · ${formatBpm(bpm)} BPM`,
  );
  const NEED_BPM = 'Type a BPM first';
  const noRemove = $derived(none || target === null || target === 0);
  const noDouble = $derived(none || bpm === null || scaleBpm(bpm, 2) === null);
  const noHalve = $derived(none || bpm === null || scaleBpm(bpm, 0.5) === null);
  const message = $derived(pl.sectionError || (segs.length === 0 && pl.status === 'active' ? NEED_BPM : ''));
</script>

<div class="flex flex-col gap-1 rounded-lg border-2 border-border bg-card p-3" role="group" aria-label="Sections">
  <span class="text-lg font-semibold">Sections</span>
  <span class="text-base tabular-nums" data-testid="section-status">{status}</span>
  <div class="flex flex-wrap items-center gap-2">
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg"
      aria-label="Add a tempo change at the playhead"
      disabled={none}
      title={none ? NEED_BPM : undefined}
      onclick={() => startAdd()}
    >Add</Button>
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg"
      aria-label="Remove the tempo change"
      disabled={noRemove}
      title={none ? NEED_BPM : noRemove ? 'The first section has no tempo change to remove' : undefined}
      onclick={() => void removeChange()}
    >Remove</Button>
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg"
      aria-label="Double the section's BPM"
      disabled={noDouble}
      title={none ? NEED_BPM : noDouble ? 'Doubling would go over 300 BPM' : undefined}
      onclick={() => void scaleSection(2)}
    >×2</Button>
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg"
      aria-label="Halve the section's BPM"
      disabled={noHalve}
      title={none ? NEED_BPM : noHalve ? 'Halving would go under 20 BPM' : undefined}
      onclick={() => void scaleSection(0.5)}
    >÷2</Button>
  </div>
  <span class="min-h-5 text-sm {pl.sectionError ? 'text-destructive' : 'text-muted-foreground'}" role={pl.sectionError ? 'alert' : undefined} data-testid="section-message">{message}</span>
</div>
