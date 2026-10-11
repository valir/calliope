<script lang="ts">
  import { selectedTrack } from '$lib/library-state.svelte';
  import { pl, selectSource } from '$lib/player-state.svelte';
  import BpmSection from './BpmSection.svelte';
  import LoopsSection from './LoopsSection.svelte';
  import MetronomeSection from './MetronomeSection.svelte';
  import PlaybackSection from './PlaybackSection.svelte';
  import WaveformView from './WaveformView.svelte';
  import SourceSelect from './SourceSelect.svelte';

  const track = $derived(selectedTrack());
  const sources = $derived(pl.snap?.sources ?? []);
</script>

<section aria-label="Player" class="flex min-h-0 min-w-0 flex-1 flex-col gap-2 overflow-y-auto p-4">
  <div class="flex flex-wrap items-center gap-x-3 gap-y-1" data-testid="player-source-row">
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
  <!-- The settings-file messages take the place of the placeholder so the layout does not shift. -->
  <div class="flex flex-1 flex-col justify-center overflow-hidden rounded-lg {pl.stateError || pl.snap?.note ? 'min-h-fit' : 'min-h-9'} border border-dashed border-border px-2 py-1" data-testid="placeholder">
    {#if pl.stateError}
      <p class="truncate text-base text-destructive" role="alert" title={pl.stateError} data-testid="player-state-error">{pl.stateError}</p>
    {/if}
    {#if pl.snap?.note}
      <p class="truncate text-base" role="status" title={pl.snap.note} data-testid="player-note">{pl.snap.note}</p>
    {/if}
    {#if !pl.stateError && !pl.snap?.note}
      <p class="truncate text-lg">The tablature will be shown here (gui-tablatures).</p>
    {/if}
  </div>
  <WaveformView />
  <div class="flex shrink-0 flex-wrap gap-2" data-testid="player-controls">
    <BpmSection />
    <MetronomeSection />
    <PlaybackSection />
    <LoopsSection />
  </div>
</section>
