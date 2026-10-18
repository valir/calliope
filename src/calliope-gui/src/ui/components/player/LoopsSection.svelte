<script lang="ts">
  import { Button } from '$lib/components/ui/button/index.js';
  import { formatBar, parseBar, formatLoopTime, parseLoopTime } from '$lib/bar-format';
  import type { Boundary, PlayerLoop } from '$lib/ipc';
  import { loop, pl } from '$lib/player-state.svelte';

  const snap = $derived(pl.snap);
  const active = $derived(pl.status === 'active' && snap !== null);
  const loops = $derived(snap?.loops ?? []);
  const cur = $derived<PlayerLoop | null>(snap && snap.current_loop !== null ? (snap.loops[snap.current_loop] ?? null) : null);
  const complete = $derived(cur !== null && cur.end !== null);
  const bpmMode = $derived(snap?.mode === 'bpm');
  const perBar = $derived(snap?.tempo.beats_per_bar ?? 4);
  const boxText = $derived(snap ? `${snap.current_loop === null ? 0 : snap.current_loop + 1}/${loops.length}` : '0/0');
  const pct = $derived(
    cur && cur.end_ms !== null && cur.end_ms > cur.start_ms
      ? Math.min(100, Math.max(0, Math.round(((pl.transport.position_ms - cur.start_ms) / (cur.end_ms - cur.start_ms)) * 100)))
      : 0,
  );
  const pressed = 'bg-amber-500! text-black! hover:bg-amber-500/90!';

  function show(b: Boundary | null, ms: number | null): string {
    if (b === null) return '';
    if (bpmMode && 'tick' in b) return formatBar(b.tick, perBar);
    return formatLoopTime(ms ?? ('ms' in b ? b.ms : 0));
  }

  // One entry per edge: unsaved text while editing, null follows the loop.
  let texts = $state<{ start: string | null; end: string | null }>({ start: null, end: null });
  let invalid = $state<{ start: boolean; end: boolean }>({ start: false, end: false });
  const shown = (edge: 'start' | 'end'): string =>
    texts[edge] ?? (cur ? (edge === 'start' ? show(cur.start, cur.start_ms) : show(cur.end, cur.end_ms)) : '');

  function commit(edge: 'start' | 'end'): void {
    const t = texts[edge];
    if (t === null) return;
    let value: Boundary | null = null;
    if (bpmMode) {
      const tick = parseBar(t, perBar);
      if (tick !== null && tick >= 0) value = { tick };
    } else {
      const ms = parseLoopTime(t);
      if (ms !== null) value = { ms };
    }
    if (value === null) {
      invalid[edge] = true;
      return;
    }
    texts[edge] = null;
    invalid[edge] = false;
    void loop({ kind: 'set', edge, value });
  }
  function revert(edge: 'start' | 'end'): void {
    texts[edge] = null;
    invalid[edge] = false;
  }
  function keydown(edge: 'start' | 'end', e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      commit(edge);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      revert(edge);
    }
  }
  function blur(edge: 'start' | 'end'): void {
    if (texts[edge] !== null && !invalid[edge]) commit(edge);
    if (invalid[edge]) revert(edge);
  }
  const flags = (repeat: boolean, count_in: boolean): void => void loop({ kind: 'flags', repeat, count_in });
  const btn = 'h-[3.25rem] text-lg';
  const box = 'w-24 rounded-md border border-input bg-background px-2 py-1 text-center text-xl tabular-nums';
</script>

<div class="flex min-w-[27rem] flex-1 flex-col gap-1 rounded-lg border-2 border-border bg-card p-3" role="group" aria-label="Loops">
  <div class="flex flex-wrap items-center gap-x-6 gap-y-2">
    <span class="text-lg font-semibold">Loops</span>
    <div class="flex flex-wrap items-center gap-2" role="group" aria-label="Loop list">
      <Button variant="outline" class={btn} disabled={!active || !(snap && snap.loops.length > 0)} onclick={() => void loop({ kind: 'select', delta: -1 })}>Prev</Button>
      <Button variant="outline" class="{btn} w-14" disabled={!active} aria-label="Add loop" onclick={() => void loop({ kind: 'add' })}>+</Button>
      <input type="text" readonly tabindex="-1" class="w-20 rounded-md border border-input bg-background px-2 py-1 text-center text-xl tabular-nums" aria-label="Loop" data-testid="loop-box" value={boxText} />
      <Button variant="outline" class="{btn} w-14" disabled={!active || !cur} aria-label="Remove loop" onclick={() => void loop({ kind: 'remove' })}>-</Button>
      <Button variant="outline" class={btn} disabled={!active || !(snap && snap.loops.length > 0)} onclick={() => void loop({ kind: 'select', delta: 1 })}>Next</Button>
    </div>
    <div class="flex flex-wrap items-center gap-2" role="group" aria-label="Loop range">
      <Button variant="outline" class="{btn} w-14" disabled={!active || !cur} aria-label="Mark start" onclick={() => void loop({ kind: 'mark', edge: 'start' })}>&gt;</Button>
      <input
        type="text" class={box} aria-label="bar1" placeholder="start" disabled={!active || !cur}
        aria-invalid={invalid.start ? 'true' : undefined} value={shown('start')}
        oninput={(e) => { texts.start = e.currentTarget.value; invalid.start = false; }}
        onkeydown={(e) => keydown('start', e)} onblur={() => blur('start')}
      />
      <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
      <div
        class="relative h-2 w-20 shrink-0 overflow-hidden rounded-full bg-muted"
        role="slider"
        aria-label="Position in the loop"
        aria-readonly="true"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-valuenow={complete ? pct : undefined}
        aria-valuetext={complete ? `${pct}%` : 'No loop end'}
        data-testid="loop-slider"
      >
        {#if complete}
          <div class="absolute inset-y-0 left-0 bg-amber-500" style:width="{pct}%"></div>
        {/if}
      </div>
      <input
        type="text" class={box} aria-label="bar2" placeholder="end" disabled={!active || !cur}
        aria-invalid={invalid.end ? 'true' : undefined} value={shown('end')}
        oninput={(e) => { texts.end = e.currentTarget.value; invalid.end = false; }}
        onkeydown={(e) => keydown('end', e)} onblur={() => blur('end')}
      />
      <Button variant="outline" class="{btn} w-14" disabled={!active || !cur} aria-label="Mark end" onclick={() => void loop({ kind: 'mark', edge: 'end' })}>&lt;</Button>
    </div>
    <div class="flex flex-wrap items-center gap-2" role="group" aria-label="Loop playback">
      <Button class="{btn} w-24" disabled={!active || !complete} onclick={() => void loop({ kind: pl.transport.loop_playing ? 'stop' : 'play' })}>
        {pl.transport.loop_playing ? 'Stop' : 'Play'}
      </Button>
      <Button variant="outline" class="{btn} {cur?.repeat && complete ? pressed : ''}" aria-pressed={(cur?.repeat && complete) ?? false} disabled={!active || !complete}
        onclick={() => cur && flags(!cur.repeat, cur.count_in)}>Loop On</Button>
      <Button variant="outline" class="{btn} {cur?.count_in ? pressed : ''}" aria-pressed={cur?.count_in ?? false} disabled={!active || !complete}
        onclick={() => cur && flags(cur.repeat, !cur.count_in)}>Count-In</Button>
    </div>
  </div>
  <span class="min-h-6 text-base {(snap?.loop_message ?? pl.actionError) ? 'text-destructive' : ''}" role="status" data-testid="loop-status">{snap?.loop_message ?? pl.actionError}</span>
</div>
