<script lang="ts">
  import { onMount } from 'svelte';
  import { frontendLog } from '$lib/ipc';
  import { selectedTrack } from '$lib/library-state.svelte';
  import {
    pl, selectSource, targetSection, selectSection, editSectionBpm, commitSectionBpm, cancelSectionBpm, startAdd, removeChange, moveChange,
  } from '$lib/player-state.svelte';
  import LoopsSection from './LoopsSection.svelte';
  import PlaybackSection from './PlaybackSection.svelte';
  import WaveformView from './WaveformView.svelte';
  import SectionsSection from './SectionsSection.svelte';
  import SourceSelect from './SourceSelect.svelte';

  const track = $derived(selectedTrack());
  const loops = $derived(pl.snap?.loops ?? []);
  const sources = $derived(pl.snap?.sources ?? []);

  // Logs the pane's overflow, width and whether Loops sits beside Sections once its size has settled.
  let pane = $state<HTMLElement | null>(null);
  onMount(() => {
    if (!pane || typeof ResizeObserver === 'undefined') return;
    const el = pane;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const report = () => {
      const top = (name: string) => el.querySelector(`[role="group"][aria-label="${name}"]`)?.getBoundingClientRect().top;
      const beside = top('Sections') !== undefined && top('Sections') === top('Loops');
      void frontendLog(`player layout overflow=${el.scrollHeight - el.clientHeight} width=${el.clientWidth} loops_beside=${beside ? 1 : 0}`);
    };
    const ro = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(report, 300);
    });
    ro.observe(el);
    const controls = el.querySelector('[data-testid="player-controls"]');
    if (controls) ro.observe(controls);
    return () => {
      clearTimeout(timer);
      ro.disconnect();
    };
  });
</script>

<section bind:this={pane} aria-label="Player" class="flex min-h-0 min-w-0 flex-1 flex-col gap-2 overflow-y-auto p-4">
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
  <WaveformView
    testid="player-waveform"
    label={`Waveform, ${loops.length} ${loops.length === 1 ? 'loop' : 'loops'}`}
    peaks={pl.snap?.peaks ?? []}
    durationMs={pl.snap?.duration_ms ?? 0}
    positionMs={pl.transport.position_ms}
    {loops}
    currentLoop={pl.snap?.current_loop ?? null}
    segments={pl.snap?.tempo.segments ?? []}
    freeMove
    selected={targetSection()}
    onselect={selectSection}
    onmove={moveChange}
    onlabel={editSectionBpm}
    onremove={(i) => { if (i > 0) void removeChange(); }}
    onadd={startAdd}
    bpmEdit={pl.bpmEdit}
    oncommitbpm={(b) => void commitSectionBpm(b)}
    oncancelbpm={cancelSectionBpm}
    oninvalidbpm={(m) => (pl.sectionError = m)}
  />
  <div class="flex shrink-0 flex-col gap-2" data-testid="player-controls">
    <div class="flex flex-wrap items-stretch gap-2">
      <SectionsSection />
      <LoopsSection />
    </div>
    <PlaybackSection />
  </div>
</section>
