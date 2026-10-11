<script lang="ts">
  import { selectedTrack } from '$lib/library-state.svelte';
  import { pl, selectSource } from '$lib/player-state.svelte';
  import SourceSelect from './SourceSelect.svelte';

  const track = $derived(selectedTrack());
  const sources = $derived(pl.snap?.sources ?? []);
</script>

<section aria-label="Player" class="flex min-h-0 min-w-0 flex-1 flex-col gap-3 p-4">
  <div class="flex items-center gap-3" data-testid="player-source-row">
    <span class="text-lg">Play:</span>
    <SourceSelect {sources} value={pl.snap?.source ?? ''} disabled={pl.status !== 'active' || sources.length === 0} onchange={(k) => void selectSource(k)} />
    <span class="min-w-0 truncate text-lg font-semibold" data-testid="player-title">{track?.title ?? ''}</span>
  </div>
  {#if pl.status === 'loading'}
    <p class="text-lg" role="status">Loading... {Math.round(pl.progress * 100)}%</p>
  {:else if pl.status === 'error'}
    <p class="text-lg text-destructive" role="alert">{pl.message}</p>
  {:else if pl.status === 'inactive' && pl.message}
    <p class="text-lg" role="status">{pl.message}</p>
  {/if}
  {#if pl.audioError}
    <p class="text-lg text-destructive" role="alert">{pl.audioError}</p>
  {/if}
  <p class="min-h-0 flex-1 rounded-lg border border-dashed border-border p-6 text-lg" data-testid="placeholder">
    The tablature will be shown here (gui-tablatures).
  </p>
  <div class="h-24 shrink-0 rounded-lg border border-border" data-testid="player-waveform"></div>
  <div class="flex shrink-0 flex-wrap gap-3" data-testid="player-controls"></div>
</section>
