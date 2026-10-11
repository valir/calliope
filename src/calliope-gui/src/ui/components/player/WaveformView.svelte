<script lang="ts">
  import { onMount } from 'svelte';
  import { pl } from '$lib/player-state.svelte';
  import { columns, loopMarks, xOf } from '$lib/waveform-geometry';

  let canvas: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let height = $state(0);

  const loops = $derived(pl.snap?.loops ?? []);
  const label = $derived(`Waveform, ${loops.length} ${loops.length === 1 ? 'loop' : 'loops'}`);

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

    const snap = pl.snap;
    const duration = snap?.duration_ms ?? 0;
    const mid = height / 2;
    ctx.fillStyle = wave;
    columns(snap?.peaks ?? [], width).forEach((peak, x) => {
      const h = Math.max(1, Math.min(1, peak) * height);
      ctx.fillRect(x, mid - h / 2, 1, h);
    });

    const marks = loopMarks(loops, duration, width);
    ctx.fillStyle = accent;
    marks.forEach((m, i) => {
      const current = i === snap?.current_loop;
      ctx.globalAlpha = current ? 1 : 0.55;
      ctx.fillRect(m.x1, 0, 2, height);
      if (m.x2 !== null) {
        ctx.fillRect(m.x2, 0, 2, height);
        ctx.fillRect(m.x1, 0, Math.max(0, m.x2 - m.x1) + 2, 4);
      }
    });
    ctx.globalAlpha = 1;

    if (duration > 0) {
      ctx.fillStyle = playhead;
      ctx.fillRect(xOf(pl.transport.position_ms, duration, width), 0, 1, height);
    }
  }

  $effect(() => {
    // Redraw on every change of what is shown.
    void [width, height, pl.snap?.peaks, pl.snap?.loops, pl.snap?.current_loop, pl.snap?.duration_ms, pl.transport.position_ms, canvas];
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
</script>

<!-- svelte-ignore a11y_no_interactive_element_to_noninteractive_role -->
<canvas
  bind:this={canvas}
  role="img"
  aria-label={label}
  class="h-20 w-full shrink-0 rounded-lg border border-border"
  data-testid="player-waveform"
></canvas>
