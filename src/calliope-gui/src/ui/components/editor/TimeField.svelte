<script lang="ts">
  import { ed, nudge, positionText, seek } from '$lib/editor-state.svelte';
  import { parsePosition } from '$lib/time-format';

  // Unsaved text while the user edits; null means the field follows the transport position.
  let text = $state<string | null>(null);
  let invalid = $state<string | null>(null);
  const shown = $derived(text ?? positionText());
  const active = $derived(ed.status === 'active');

  function commit(): void {
    if (text === null) return;
    const ms = parsePosition(text);
    if (ms === null || ms > ed.durationMs) {
      invalid = ms === null ? 'Enter a time like 1:23.4' : 'That time is past the end of the track';
      return;
    }
    text = null;
    invalid = null;
    seek(ms);
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
    if (text === null) return;
    commit();
    if (invalid !== null) revert();
  }
  function onwheel(e: WheelEvent): void {
    e.preventDefault();
    if (!active || e.deltaY === 0) return;
    revert();
    void nudge(e.deltaY < 0 ? -100 : 100);
  }
</script>

<div class="flex w-32 shrink-0 flex-col items-center">
  <input
    type="text"
    class="w-full rounded-md border border-input bg-background px-2 py-1 text-center text-2xl font-semibold tabular-nums"
    data-testid="time-text"
    aria-label="Position (minutes:seconds.tenths)"
    aria-invalid={invalid !== null ? 'true' : undefined}
    disabled={!active}
    value={shown}
    oninput={(e) => { text = e.currentTarget.value; invalid = null; }}
    {onkeydown}
    {onblur}
    {onwheel}
  />
  {#if invalid}
    <span class="text-sm text-destructive" role="alert" data-testid="time-error">{invalid}</span>
  {/if}
</div>
