<script lang="ts">
  import { onMount } from 'svelte';
  import type { PlayerLoop, TempoSegment } from '$lib/ipc';
  import {
    columns, dragSteps, fineMs, labelFits, loopMarks, MAX_OFFSET_STEPS, moveLimit, moveRefMs, offsetSteps, sectionMarks, xOf,
  } from '$lib/waveform-geometry';
  import SectionBpmInput from './SectionBpmInput.svelte';

  interface Props {
    peaks: number[];
    durationMs: number;
    positionMs: number;
    loops?: PlayerLoop[];
    currentLoop?: number | null;
    segments?: TempoSegment[];
    /** Moves detected change `index` to the absolute offset `steps`; without it the view is read-only. */
    onmove?: (index: number, steps: number) => void;
    /** Drag and keys are off while true (a detection is running). */
    locked?: boolean;
    testid: string;
    /** Start of the accessible name; the tempos are appended. */
    label?: string;
    class?: string;
    /** Player mode: every change can move (`onmove` gets the offset from `moveRefMs`), and the selection is controlled. */
    freeMove?: boolean;
    /** The target section (freeMove only). */
    selected?: number | null;
    onselect?: (index: number) => void;
    /** The label of section `index` was clicked, or Enter was pressed on it. */
    onlabel?: (index: number) => void;
    onremove?: (index: number) => void;
    onadd?: () => void;
    /** The BPM overlay to show: over a section's label, or at a playhead position. */
    bpmEdit?: { index: number } | { addAt: number } | null;
    oncommitbpm?: (bpm: number) => void;
    oncancelbpm?: () => void;
    oninvalidbpm?: (message: string) => void;
  }
  let {
    peaks, durationMs, positionMs, loops = [], currentLoop = null, segments = [], onmove, locked = false,
    testid, label = 'Waveform', class: cls = 'h-20',
    freeMove = false, selected = null, onselect, onlabel, onremove, onadd,
    bpmEdit = null, oncommitbpm, oncancelbpm, oninvalidbpm,
  }: Props = $props();

  const GRAB_PX = 8;
  const FONT = '600 15px sans-serif';

  let canvas: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);

  /** Index of the change being dragged and its previewed offset. */
  let drag = $state<{ index: number; steps: number } | null>(null);
  let focused = $state(false);
  let selectedInt = $state<number | null>(null);
  /** Where the labels were drawn last (for the click hit-test). */
  let labelRects: { index: number; x: number; y: number; w: number; h: number }[] = [];
  /** The offset last asked for by the keys (the segments update only after the backend answers). */
  let keyTarget: { index: number; steps: number } | null = null;
  let hoverTyped = $state(false);
  let hoverBar = $state(false);

  // the sections can shrink mid-drag (a new tempo map arrives): drop the drag
  $effect(() => {
    if (drag && drag.index >= segments.length) drag = null;
  });

  const editable = $derived(!!onmove);
  const shown = $derived(
    drag
      ? segments.map((s, i) => (i === drag!.index ? { ...s, start_ms: moveRefMs(segments, i) + drag!.steps * fineMs(segments[i - 1].bpm) } : s))
      : segments,
  );
  const detectedIdx = $derived(segments.flatMap((s, i) => (i > 0 && s.detected_ms != null ? [i] : [])));
  const active = $derived(
    editable && !locked && (freeMove ? segments.length > 0 && segments[0].bpm > 0 : detectedIdx.length > 0),
  );
  const sel = $derived(
    freeMove
      ? Math.min(Math.max(selected ?? 0, 0), Math.max(segments.length - 1, 0))
      : selectedInt !== null && detectedIdx.includes(selectedInt) ? selectedInt : (detectedIdx[0] ?? null),
  );

  function fmt(ms: number, decimals: 1 | 3): string {
    const total = Math.max(0, ms) / 1000;
    const m = Math.floor(total / 60);
    const s = (total - m * 60).toFixed(decimals);
    return `${m}:${Number(s) < 10 ? '0' : ''}${s}`;
  }
  const bpmText = (bpm: number): string => `${Number(bpm.toFixed(2))}`;
  const signed = (n: number): string => (n > 0 ? `+${n}` : `${n}`);

  const ariaLabel = $derived.by(() => {
    const parts = segments
      .filter((s) => s.bpm > 0)
      .map((s, i) => `${bpmText(s.bpm)} BPM ${i === 0 ? 'from the start' : `from ${fmt(s.start_ms, 1)}`}`);
    return parts.length ? `${label}, tempo ${parts.join(', ')}` : label;
  });

  function offsetOf(i: number): number {
    return offsetSteps(segments[i].start_ms, moveRefMs(segments, i), segments[i - 1].bpm);
  }
  const valueText = $derived.by(() => {
    if (!active || sel === null) return undefined;
    if (freeMove) {
      const s = segments[sel];
      const nm = s.name ? `, ${s.name},` : '';
      const head = `Section ${sel + 1} of ${segments.length}${nm} ${sel === 0 ? 'from the start' : `from ${fmt(s.start_ms, 3)}`}, ${bpmText(s.bpm)} BPM`;
      if (sel === 0) return head;
      const kind = s.detected_ms != null ? 'detected' : s.on_beat ? 'placed' : 'typed';
      return kind === 'typed' ? `${head}, typed` : `${head}, ${kind}, ${signed(offsetOf(sel))}/64 beat`;
    }
    const n = offsetOf(sel);
    const pos = detectedIdx.indexOf(sel) + 1;
    return `Tempo change ${pos} of ${detectedIdx.length} at ${fmt(segments[sel].start_ms, 3)}, ${bpmText(segments[sel].bpm)} BPM, ${signed(n)}/64 beat`;
  });
  const readout = $derived.by(() => {
    if (!drag || drag.index >= segments.length) return '';
    const ms = drag.steps * fineMs(segments[drag.index - 1].bpm);
    const sign = ms < 0 ? '-' : '+';
    const limit = freeMove ? moveLimit(segments, drag.index) : MAX_OFFSET_STEPS;
    const tail = Number.isFinite(limit) ? ` · limit ±${limit / 64} beats` : '';
    return `${signed(drag.steps)}/64 beat (${sign}${fmt(Math.abs(ms), 3)})${tail}`;
  });

  /** Neighbour limits of change `i` in ms, as `set_change_offset`. */
  function limits(i: number): [number, number] {
    const prev = segments[i - 1];
    const next = segments[i + 1];
    return [prev.start_ms + 60000 / prev.bpm, next ? next.start_ms - 60000 / segments[i].bpm : Infinity];
  }
  function stepsFor(i: number, pointerMs: number): number {
    const [lo, hi] = limits(i);
    const limit = freeMove ? moveLimit(segments, i) : MAX_OFFSET_STEPS;
    return dragSteps(moveRefMs(segments, i), pointerMs, segments[i - 1].bpm, lo, hi, limit);
  }

  function colour(style: CSSStyleDeclaration, name: string, fallback: string): string {
    return style.getPropertyValue(name).trim() || fallback;
  }

  function draw(): void {
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx || width <= 0 || height <= 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);

    const style = getComputedStyle(canvas);
    const wave = colour(style, '--muted-foreground', '#888');
    const accent = colour(style, '--primary', '#f90');
    const playhead = colour(style, '--foreground', '#000');

    const duration = durationMs;
    const mid = height / 2;
    ctx.fillStyle = wave;
    columns(peaks, width).forEach((peak, x) => {
      const h = Math.max(1, Math.min(1, peak) * height);
      ctx.fillRect(x, mid - h / 2, 1, h);
    });

    const marks = loopMarks(loops, duration, width);
    ctx.fillStyle = accent;
    marks.forEach((m, i) => {
      const current = i === currentLoop;
      ctx.globalAlpha = current ? 1 : 0.55;
      ctx.fillRect(m.x1, 0, 2, height);
      if (m.x2 !== null) {
        ctx.fillRect(m.x2, 0, 2, height);
        ctx.fillRect(m.x1, 0, Math.max(0, m.x2 - m.x1) + 2, 4);
      }
    });
    ctx.globalAlpha = 1;

    drawSections(ctx, accent, playhead, duration);

    if (duration > 0) {
      ctx.fillStyle = playhead;
      ctx.fillRect(xOf(positionMs, duration, width), 0, 1, height);
    }
  }

  function drawSections(ctx: CanvasRenderingContext2D, accent: string, text: string, duration: number): void {
    const secs = sectionMarks(shown, duration, width);
    secs.forEach((m) => {
      if (m.x === null) return;
      const isSel = freeMove ? active && m.index === sel : active && focused && m.index === sel;
      ctx.globalAlpha = editable && !freeMove && !m.detected ? 0.55 : 1;
      ctx.fillStyle = accent;
      if (isSel) ctx.fillRect(m.x - 1, 0, 4, height);
      else ctx.fillRect(m.x, 0, 2, height);
    });
    ctx.globalAlpha = 1;

    ctx.font = FONT;
    ctx.textBaseline = 'alphabetic';
    ctx.fillStyle = text;
    labelRects = [];
    secs.forEach((m, i) => {
      if (m.bpm <= 0) return;
      const x = m.x === null ? 3 : m.x + 5;
      const nextX = secs[i + 1]?.x ?? null;
      const b = bpmText(m.bpm);
      for (const t of [`${b} BPM`, b]) {
        if (labelFits(x, nextX, ctx.measureText(t).width)) {
          const w = ctx.measureText(t).width;
          ctx.fillText(t, x, height - 4);
          labelRects.push({ index: m.index, x, y: height - 19, w, h: 19 });
          if (freeMove && active && m.index === sel) ctx.fillRect(x, height - 2, w, 2);
          break;
        }
      }
    });
  }

  $effect(() => {
    // Redraw on every change of what is shown.
    void [width, height, peaks, loops, currentLoop, durationMs, positionMs, shown, canvas, focused, sel, active, editable, freeMove];
    draw();
  });

  onMount(() => {
    if (!canvas) return;
    const measure = () => {
      width = Math.floor(canvas!.clientWidth);
      height = Math.floor(canvas!.clientHeight);
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(measure);
    ro.observe(canvas);
    return () => ro.disconnect();
  });

  // ---- pointer ----

  const localX = (e: PointerEvent): number => e.clientX - (canvas?.getBoundingClientRect().left ?? 0);
  const msAt = (x: number): number => (width > 1 ? (Math.min(Math.max(x, 0), width - 1) / (width - 1)) * durationMs : 0);

  /** The change (index >= 1) whose bar is within GRAB_PX of `x`, nearest first. */
  function barAt(x: number): number | null {
    let best: number | null = null;
    let bestD = GRAB_PX + 1;
    sectionMarks(segments, durationMs, width).forEach((m) => {
      if (m.x === null) return;
      const d = Math.abs(m.x + 1 - x);
      if (d <= GRAB_PX && d < bestD) { best = m.index; bestD = d; }
    });
    return best;
  }

  const localY = (e: PointerEvent): number => e.clientY - (canvas?.getBoundingClientRect().top ?? 0);

  function labelAt(x: number, y: number): number | null {
    const r = labelRects.find((l) => x >= l.x && x <= l.x + l.w && y >= l.y && y <= l.y + l.h);
    return r ? r.index : null;
  }

  /** The section the pixel `x` falls in. */
  function sectionAt(x: number): number {
    let at = 0;
    sectionMarks(segments, durationMs, width).forEach((m) => {
      if (m.x !== null && m.x <= x) at = m.index;
    });
    return at;
  }

  function onpointerdown(e: PointerEvent): void {
    if (!editable || locked || e.button > 0) return;
    const x = localX(e);
    if (freeMove) {
      if (!active) return;
      e.preventDefault();
      canvas?.focus();
      keyTarget = null;
      const l = labelAt(x, localY(e));
      if (l !== null) {
        onlabel?.(l);
        return;
      }
      const b = barAt(x);
      if (b === null) {
        onselect?.(sectionAt(x));
        return;
      }
      drag = { index: b, steps: offsetOf(b) };
      onselect?.(b);
      canvas?.setPointerCapture?.(e.pointerId);
      return;
    }
    const i = barAt(x);
    if (i === null || segments[i].detected_ms == null) return;
    drag = { index: i, steps: offsetOf(i) };
    selectedInt = i;
    keyTarget = null;
    canvas?.setPointerCapture?.(e.pointerId);
    e.preventDefault();
  }

  function onpointermove(e: PointerEvent): void {
    if (!editable) return;
    if (drag) {
      drag = { index: drag.index, steps: stepsFor(drag.index, msAt(localX(e))) };
      return;
    }
    const i = locked ? null : barAt(localX(e));
    hoverBar = i !== null && (freeMove || segments[i].detected_ms != null);
    hoverTyped = !freeMove && i !== null && segments[i].detected_ms == null;
  }

  function onpointerup(): void {
    if (!drag) return;
    const { index, steps } = drag;
    drag = null;
    if (steps !== offsetOf(index)) onmove?.(index, steps);
  }

  function onwindowkeydown(e: KeyboardEvent): void {
    if (drag && e.key === 'Escape') {
      drag = null;
      e.preventDefault();
    }
  }

  // ---- keys ----

  function onfocus(): void {
    focused = true;
    if (!freeMove) selectedInt = detectedIdx[0] ?? null;
    keyTarget = null;
  }

  function onblur(): void {
    focused = false;
    keyTarget = null;
  }

  function onkeydown(e: KeyboardEvent): void {
    if (!active || sel === null || drag || e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      if (freeMove) {
        onselect?.(Math.min(Math.max(sel + (e.key === 'ArrowUp' ? -1 : 1), 0), segments.length - 1));
      } else {
        const at = detectedIdx.indexOf(sel) + (e.key === 'ArrowUp' ? -1 : 1);
        selectedInt = detectedIdx[Math.min(Math.max(at, 0), detectedIdx.length - 1)];
      }
      keyTarget = null;
      e.preventDefault();
      return;
    }
    if (freeMove && !e.shiftKey) {
      if (e.key === 'Enter') {
        e.preventDefault();
        onlabel?.(sel);
        return;
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        onremove?.(sel);
        return;
      }
      if (e.key === 'a') {
        e.preventDefault();
        onadd?.();
        return;
      }
    }
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    if (sel === 0) return;
    const delta = (e.key === 'ArrowRight' ? 1 : -1) * (e.shiftKey ? 64 : 1);
    const base = keyTarget && keyTarget.index === sel ? keyTarget.steps : offsetOf(sel);
    const limit = freeMove ? moveLimit(segments, sel) : MAX_OFFSET_STEPS;
    const wanted = Math.min(Math.max(base + delta, -limit), limit);
    const fine = fineMs(segments[sel - 1].bpm);
    const steps = stepsFor(sel, moveRefMs(segments, sel) + wanted * fine);
    if (steps === base) return;
    keyTarget = { index: sel, steps };
    onmove?.(sel, steps);
  }

  // ---- BPM overlay ----

  const overlay = $derived.by(() => {
    if (!bpmEdit || width <= 0 || segments.length === 0) return null;
    let x: number;
    let initial: number;
    let name: string;
    if ('addAt' in bpmEdit) {
      x = xOf(bpmEdit.addAt, durationMs, width);
      initial = segments.filter((s, i) => i === 0 || s.start_ms <= bpmEdit.addAt).at(-1)!.bpm;
      name = 'BPM of the new section';
    } else {
      const i = bpmEdit.index;
      if (i < 0 || i >= segments.length) return null;
      x = i === 0 ? 3 : xOf(segments[i].start_ms, durationMs, width) + 5;
      initial = segments[i].bpm;
      name = `BPM of section ${i + 1}`;
    }
    return { x: Math.max(0, Math.min(x, width - 100)), initial, name };
  });

  function commitBpm(bpm: number): void {
    oncommitbpm?.(bpm);
    canvas?.focus();
  }
  function cancelBpm(): void {
    // Escape (the input still has the focus) hands the focus back; a blur must not take it
    const byKey = document.activeElement?.hasAttribute('data-bpm-input');
    oncancelbpm?.();
    if (byKey) canvas?.focus();
  }

  const title = $derived(
    hoverTyped ? 'This tempo change was typed in the Player; only detected changes can be moved' : undefined,
  );
</script>

<svelte:window onkeydown={onwindowkeydown} />

<div class="relative shrink-0">
  <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role, a11y_no_noninteractive_tabindex, a11y_no_interactive_element_to_noninteractive_role -->
  <canvas
    bind:this={canvas}
    role={active ? 'slider' : 'img'}
    aria-label={ariaLabel}
    aria-valuemin={active && !freeMove ? -MAX_OFFSET_STEPS : undefined}
    aria-valuemax={active && !freeMove ? MAX_OFFSET_STEPS : undefined}
    aria-valuenow={active && sel !== null ? (sel === 0 ? 0 : offsetOf(sel)) : undefined}
    data-player-keys={freeMove ? '' : undefined}
    aria-valuetext={valueText}
    tabindex={active ? 0 : undefined}
    {title}
    class="{cls} w-full rounded-lg border border-border {drag || hoverBar ? 'cursor-ew-resize' : ''} focus-visible:outline-2 focus-visible:outline-ring"
    data-testid={testid}
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={() => (drag = null)}
    {onkeydown}
    {onfocus}
    {onblur}
  ></canvas>
  {#if overlay}
    <SectionBpmInput
      xPx={overlay.x}
      initial={overlay.initial}
      label={overlay.name}
      oncommit={commitBpm}
      oncancel={cancelBpm}
      oninvalid={(m) => oninvalidbpm?.(m)}
    />
  {/if}
  {#if drag}
    <div class="pointer-events-none absolute top-1 left-1 rounded bg-background/90 px-2 py-0.5 text-base" data-testid="{testid}-readout">{readout}</div>
  {/if}
</div>
