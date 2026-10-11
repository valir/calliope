<script lang="ts">
  import { selectedTrack } from '$lib/library-state.svelte';
  import SourceSelect from './SourceSelect.svelte';

  // Task 13 wires the sources and the transport; this is the layout.
  const track = $derived(selectedTrack());
  const sources: { key: string; label: string }[] = [];
</script>

<section aria-label="Player" class="flex min-h-0 min-w-0 flex-1 flex-col gap-3 p-4">
  <div class="flex items-center gap-3" data-testid="player-source-row">
    <span class="text-lg">Play:</span>
    <SourceSelect {sources} value="" disabled={sources.length === 0} onchange={() => {}} />
    <span class="min-w-0 truncate text-lg font-semibold" data-testid="player-title">{track?.title ?? ''}</span>
  </div>
  <p class="min-h-0 flex-1 rounded-lg border border-dashed border-border p-6 text-lg" data-testid="placeholder">
    The tablature will be shown here (gui-tablatures).
  </p>
  <div class="h-24 shrink-0 rounded-lg border border-border" data-testid="player-waveform"></div>
  <div class="flex shrink-0 flex-wrap gap-3" data-testid="player-controls"></div>
</section>
