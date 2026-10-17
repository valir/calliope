<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatBpm, scaleBpm } from '$lib/bpm-format';
  import { sectionClicks, sectionCountIn, MAX_SECTION_NAME } from '$lib/section-meta';
  import {
    pl, removeChange, scaleSection, startAdd, targetSection, setMetronome, setSectionFlags, jumpSection,
    beginNameEdit, commitSectionName, cancelSectionName,
  } from '$lib/player-state.svelte';

  const segs = $derived(pl.snap?.tempo.segments ?? []);
  const none = $derived(pl.status !== 'active' || segs.length === 0);
  // While the name box has the focus, everything shows the section captured on focus.
  const target = $derived(none ? null : pl.nameEdit ? Math.min(pl.nameEdit.index, segs.length - 1) : targetSection());
  const bpm = $derived(target === null ? null : segs[target].bpm);
  const status = $derived(
    target === null || bpm === null ? 'No tempo' : `Section ${target + 1} of ${segs.length} · ${formatBpm(bpm)} BPM`,
  );
  const NEED_BPM = 'Type a BPM first';
  const noRemove = $derived(none || target === null || target === 0);
  const noDouble = $derived(none || bpm === null || scaleBpm(bpm, 2) === null);
  const noHalve = $derived(none || bpm === null || scaleBpm(bpm, 0.5) === null);
  const meta = $derived(target === null ? null : segs[target]);
  const clicks = $derived(meta ? sectionClicks(meta) : false);
  const countIn = $derived(meta ? sectionCountIn(meta) : false);
  const noCountIn = $derived(none || target === null || target === 0);
  const usable = $derived(pl.status === 'active' && pl.snap?.mode === 'bpm');
  const mOn = $derived(pl.snap?.metronome.on ?? false);
  const mCountIn = $derived(pl.snap?.metronome.count_in ?? false);
  const pressed = 'bg-amber-500! text-black! hover:bg-amber-500/90!';
  const nameValue = $derived(pl.nameEdit ? pl.nameEdit.draft : (meta?.name ?? ''));
  const message = $derived(pl.sectionError || (segs.length === 0 && pl.status === 'active' ? NEED_BPM : ''));
</script>

<div class="flex flex-col gap-1 rounded-lg border-2 border-border bg-card p-3" role="group" aria-label="Sections">
  <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
    <span class="text-lg font-semibold">Sections</span>
    <span class="text-base tabular-nums" data-testid="section-status">{status}</span>
    <input
      type="text"
      class="h-9 w-48 rounded-md border border-input bg-background px-2 text-base disabled:opacity-50"
      aria-label="Section name"
      placeholder="Name this section"
      maxlength={MAX_SECTION_NAME}
      disabled={none}
      title={none ? NEED_BPM : undefined}
      value={nameValue}
      onfocus={beginNameEdit}
      oninput={(e) => { if (pl.nameEdit) pl.nameEdit.draft = e.currentTarget.value; }}
      onblur={() => void commitSectionName()}
      onkeydown={(e) => {
        if (e.key === 'Enter') { e.preventDefault(); e.currentTarget.blur(); }
        else if (e.key === 'Escape') { e.preventDefault(); cancelSectionName(); e.currentTarget.blur(); }
      }}
    />
  </div>
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
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg"
      aria-label="Jump to the start of the section"
      disabled={none}
      title={none ? NEED_BPM : undefined}
      onclick={() => void jumpSection('start')}
    >|◀ Start</Button>
    <Button
      variant="outline"
      class="h-[3.25rem] text-lg"
      aria-label="Jump to the last bar of the section"
      disabled={none}
      title={none ? NEED_BPM : undefined}
      onclick={() => void jumpSection('end')}
    >End ▶|</Button>
  </div>
  <div class="flex flex-wrap items-center gap-x-4 gap-y-1">
    <div class="flex items-center gap-2" role="group" aria-label="Metronome">
      <span class="text-base">Metronome</span>
      <Button
        variant="outline"
        class="h-[3.25rem] text-lg {mOn ? pressed : ''}"
        aria-pressed={mOn}
        disabled={!usable}
        onclick={() => void setMetronome(!mOn, mCountIn)}
      >On/Off</Button>
      <Button
        variant="outline"
        class="h-[3.25rem] text-lg {mCountIn ? pressed : ''}"
        aria-pressed={mCountIn}
        disabled={!usable}
        onclick={() => void setMetronome(mOn, !mCountIn)}
      >Count In</Button>
    </div>
    <div class="flex items-center gap-2" role="group" aria-label="This section">
      <span class="text-base">This section</span>
      <Button
        variant="outline"
        class="h-[3.25rem] text-lg {clicks && !none ? pressed : ''}"
        aria-label="Metronome in this section"
        aria-pressed={clicks && !none}
        disabled={none}
        title={none ? NEED_BPM : undefined}
        onclick={() => void setSectionFlags(!clicks, countIn)}
      >Click</Button>
      <Button
        variant="outline"
        class="h-[3.25rem] text-lg {countIn && !noCountIn ? pressed : ''}"
        aria-label="Count-in into this section"
        aria-pressed={countIn && !noCountIn}
        disabled={noCountIn}
        title={none ? NEED_BPM : noCountIn ? 'The first section has no section before it' : undefined}
        onclick={() => void setSectionFlags(clicks, !countIn)}
      >Count In</Button>
    </div>
  </div>
  <span class="min-h-5 text-sm {pl.sectionError ? 'text-destructive' : 'text-muted-foreground'}" role={pl.sectionError ? 'alert' : undefined} data-testid="section-message">{message}</span>
</div>
