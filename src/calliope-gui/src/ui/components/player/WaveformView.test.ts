import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import WaveformView from './WaveformView.svelte';
import { pl, resetPlayer } from '$lib/player-state.svelte';
import { columns, loopMarks } from '$lib/waveform-geometry';
import type { PlayerLoop } from '$lib/ipc';

const mk = (id: string, start_ms: number, end_ms: number | null): PlayerLoop => ({
  id, start: { ms: start_ms }, end: end_ms === null ? null : { ms: end_ms }, start_ms, end_ms, repeat: false, count_in: false,
});
const peaks = Array.from({ length: 50 }, (_, i) => (i % 10) / 10);

let rects: { x: number; y: number; w: number; h: number }[];
let observers: (() => void)[];
let cssWidth = 200;

beforeEach(() => {
  rects = [];
  observers = [];
  cssWidth = 200;
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((() => ({
    setTransform() {}, clearRect() { rects = []; }, fillRect: (x: number, y: number, w: number, h: number) => rects.push({ x, y, w, h }),
  })) as never);
  Object.defineProperty(HTMLCanvasElement.prototype, 'clientWidth', { configurable: true, get: () => cssWidth });
  Object.defineProperty(HTMLCanvasElement.prototype, 'clientHeight', { configurable: true, get: () => 96 });
  vi.stubGlobal('ResizeObserver', class {
    constructor(cb: () => void) { observers.push(cb); }
    observe() {}
    disconnect() {}
  });
  resetPlayer();
  pl.snap = {
    id: 't', title: 'T', sources: [], source: 's', duration_ms: 20000, sample_rate: 44100, peaks, mode: 'time',
    tempo: { beats_per_bar: 4, segments: [], multi: false }, speed: 1, metronome: { on: false, count_in: false },
    loops: [mk('loop-1', 2000, 10000), mk('loop-2', 3000, 4000), mk('loop-3', 1000, null)], current_loop: 0,
    loop_message: null, can_change_speed: false, note: null,
  } as never;
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); resetPlayer(); });

const waveRects = () => rects.filter((r) => r.w === 1 && r.h < 96);

describe('WaveformView', () => {
  it('draws columns for the measured width and redraws on resize', async () => {
    render(WaveformView);
    await tick();
    const expected = columns(peaks, 200);
    expect(waveRects().length).toBe(expected.length);
    cssWidth = 120;
    observers.forEach((cb) => cb());
    await tick();
    expect(waveRects().length).toBe(columns(peaks, 120).length);
  });

  it('draws loop marks at the loopMarks positions', async () => {
    render(WaveformView);
    await tick();
    const [m1, , m3] = loopMarks(pl.snap!.loops, 20000, 200);
    const lines = (x: number) => rects.filter((r) => r.w === 2 && r.h === 96 && r.x === x);
    expect(lines(m1.x1).length).toBe(1);
    expect(lines(m1.x2!).length).toBe(1);
    expect(rects.some((r) => r.x === m1.x1 && r.h === 4 && r.w === m1.x2! - m1.x1 + 2)).toBe(true);
    expect(lines(m3.x1).length).toBe(1);
    expect(m3.x2).toBeNull();
  });

  it('labels the loop count', async () => {
    render(WaveformView);
    expect(screen.getByRole('img', { name: 'Waveform, 3 loops' })).toBeTruthy();
  });
});
