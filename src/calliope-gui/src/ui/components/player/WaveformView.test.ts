import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import WaveformView from './WaveformView.svelte';
import { columns, fineMs, loopMarks, sectionMarks } from '$lib/waveform-geometry';
import type { PlayerLoop, TempoSegment } from '$lib/ipc';

const mk = (id: string, start_ms: number, end_ms: number | null): PlayerLoop => ({
  id, start: { ms: start_ms }, end: end_ms === null ? null : { ms: end_ms }, start_ms, end_ms, repeat: false, count_in: false,
});
const peaks = Array.from({ length: 50 }, (_, i) => (i % 10) / 10);

let rects: { x: number; y: number; w: number; h: number }[];
let texts: { t: string; x: number; y: number }[];
let observers: (() => void)[];
let cssWidth = 200;

beforeEach(() => {
  rects = [];
  texts = [];
  observers = [];
  cssWidth = 200;
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation((() => ({
    setTransform() {}, clearRect() { rects = []; texts = []; }, measureText: (t: string) => ({ width: t.length * 6 }),
    fillText: (t: string, x: number, y: number) => texts.push({ t, x, y }), fillRect: (x: number, y: number, w: number, h: number) => rects.push({ x, y, w, h }),
  })) as never);
  Object.defineProperty(HTMLCanvasElement.prototype, 'clientWidth', { configurable: true, get: () => cssWidth });
  Object.defineProperty(HTMLCanvasElement.prototype, 'clientHeight', { configurable: true, get: () => 96 });
  vi.stubGlobal('ResizeObserver', class {
    constructor(cb: () => void) { observers.push(cb); }
    observe() {}
    disconnect() {}
  });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

const loops = [mk('loop-1', 2000, 10000), mk('loop-2', 3000, 4000), mk('loop-3', 1000, null)];
const base = { testid: 'wf', peaks, durationMs: 20000, positionMs: 0, label: 'Waveform, 3 loops' };
const withLoops = { ...base, loops, currentLoop: 0 };
const waveRects = () => rects.filter((r) => r.w === 1 && r.h < 96);

describe('WaveformView', () => {
  it('draws columns for the measured width and redraws on resize', async () => {
    render(WaveformView, { props: withLoops });
    await tick();
    const expected = columns(peaks, 200);
    expect(waveRects().length).toBe(expected.length);
    cssWidth = 120;
    observers.forEach((cb) => cb());
    await tick();
    expect(waveRects().length).toBe(columns(peaks, 120).length);
  });

  it('draws loop marks at the loopMarks positions', async () => {
    render(WaveformView, { props: withLoops });
    await tick();
    const [m1, , m3] = loopMarks(loops, 20000, 200);
    const lines = (x: number) => rects.filter((r) => r.w === 2 && r.h === 96 && r.x === x);
    expect(lines(m1.x1).length).toBe(1);
    expect(lines(m1.x2!).length).toBe(1);
    expect(rects.some((r) => r.x === m1.x1 && r.h === 4 && r.w === m1.x2! - m1.x1 + 2)).toBe(true);
    expect(lines(m3.x1).length).toBe(1);
    expect(m3.x2).toBeNull();
  });

  it('labels the loop count', async () => {
    render(WaveformView, { props: withLoops });
    expect(screen.getByRole('img', { name: 'Waveform, 3 loops' })).toBeTruthy();
  });

  it('without segments the Player render has no section bars or labels', async () => {
    render(WaveformView, { props: withLoops });
    await tick();
    expect(texts).toEqual([]);
    expect(rects.filter((r) => r.w === 2 && r.h === 96).length).toBe(5);
  });
});

const seg = (start_ms: number, bpm: number, detected_ms?: number): TempoSegment => ({ start_ms, bpm, ...(detected_ms === undefined ? {} : { detected_ms }) });
// 120 BPM -> 90 BPM at 6500 (detected 6508) -> 100 BPM at 12000 (typed); fine step 7.8125 ms at 120 BPM
const segs = [seg(0, 120), seg(6500, 90, 6500), seg(12000, 100)];
const bars = () => rects.filter((r) => r.h === 96 && r.w === 2);

describe('WaveformView sections', () => {
  it('draws change bars at the section positions with labels at the bottom', async () => {
    render(WaveformView, { props: { ...base, segments: segs } });
    await tick();
    const [m0, m1, m2] = sectionMarks(segs, 20000, 200);
    expect(m0.x).toBeNull();
    expect(bars().map((r) => r.x)).toEqual([m1.x, m2.x]);
    expect(texts).toEqual([
      { t: '120 BPM', x: 3, y: 92 },
      { t: '90 BPM', x: m1.x! + 5, y: 92 },
      { t: '100 BPM', x: m2.x! + 5, y: 92 },
    ]);
  });

  it('shortens a label that does not fit and omits one that cannot', async () => {
    // bars at x=30 and x=31: "120 BPM" (42px) from x=3 does not fit, "120" does; the 90 label has no room
    render(WaveformView, { props: { ...base, durationMs: 200000, positionMs: 0, segments: [seg(0, 120), seg(30000, 90), seg(31000, 100)] } });
    await tick();
    expect(texts.map((t) => t.t)).toEqual(['120', '100 BPM']);
    expect(texts.length).toBe(2);
  });

  it('draws no labels for BPM 0', async () => {
    render(WaveformView, { props: { ...base, segments: [seg(0, 0)] } });
    await tick();
    expect(texts).toEqual([]);
  });

  it('lists every tempo in the aria-label', () => {
    render(WaveformView, { props: { ...base, segments: segs } });
    expect(screen.getByTestId('wf').getAttribute('aria-label')).toBe(
      'Waveform, 3 loops, tempo 120 BPM from the start, 90 BPM from 0:06.5, 100 BPM from 0:12.0',
    );
  });

  it('is read-only without onmove', async () => {
    render(WaveformView, { props: { ...base, segments: segs } });
    const c = screen.getByTestId('wf');
    expect(c.getAttribute('role')).toBe('img');
    expect(c.getAttribute('tabindex')).toBeNull();
    await fireEvent.pointerDown(c, { clientX: sectionMarks(segs, 20000, 200)[1].x! });
    await fireEvent.pointerUp(c);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
  });
});

describe('WaveformView editing', () => {
  // 3 changes: detected, typed, detected
  const edit = [seg(0, 120), seg(6508, 90, 6500), seg(10000, 100), seg(15000, 110, 15000)];
  let moves: [number, number][];
  const mount = (extra: Record<string, unknown> = {}) => {
    moves = [];
    render(WaveformView, { props: { ...base, segments: edit, onmove: (i: number, s: number) => moves.push([i, s]), ...extra } });
    return screen.getByTestId('wf');
  };
  const barX = (i: number) => sectionMarks(edit, 20000, 200)[i].x!;
  const pxMs = 20000 / 199;

  it('draws typed changes at 55 % opacity: the slider role needs a detected change', async () => {
    const c = mount();
    expect(c.getAttribute('role')).toBe('slider');
    expect(c.getAttribute('tabindex')).toBe('0');
    cleanup();
    const typed = mount({ segments: [seg(0, 120), seg(6500, 90)] });
    expect(typed.getAttribute('role')).toBe('img');
  });

  it('a drag of N px sends one onmove with the absolute steps', async () => {
    const c = mount();
    const x0 = barX(1);
    await fireEvent.pointerDown(c, { clientX: x0 + 1 });
    await fireEvent.pointerMove(c, { clientX: x0 + 5 });
    const preview = screen.getByTestId('wf-readout').textContent!;
    expect(preview).toMatch(/^\+\d+\/64 beat \(\+0:00\.\d{3}\) · limit ±4 beats$/);
    await fireEvent.pointerUp(c);
    expect(moves.length).toBe(1);
    const expected = Math.round(((x0 + 5) * pxMs - 6500) / fineMs(120));
    expect(moves[0][0]).toBe(1);
    expect(Math.abs(moves[0][1] - expected)).toBeLessThanOrEqual(1);
    expect(moves[0][1]).toBeLessThanOrEqual(256);
    expect(screen.queryByTestId('wf-readout')).toBeNull();
  });

  it('Escape cancels a drag', async () => {
    const c = mount();
    await fireEvent.pointerDown(c, { clientX: barX(1) });
    await fireEvent.pointerMove(c, { clientX: barX(1) + 4 });
    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.pointerUp(c);
    expect(moves).toEqual([]);
    expect(screen.queryByTestId('wf-readout')).toBeNull();
  });

  it('typed changes do not start a drag', async () => {
    const c = mount();
    await fireEvent.pointerDown(c, { clientX: barX(2) });
    await fireEvent.pointerMove(c, { clientX: barX(2) + 6 });
    expect(screen.queryByTestId('wf-readout')).toBeNull();
    await fireEvent.pointerUp(c);
    expect(moves).toEqual([]);
  });

  it('a pointer more than 8 px from a bar starts nothing', async () => {
    const c = mount();
    await fireEvent.pointerDown(c, { clientX: barX(1) + 30 });
    await fireEvent.pointerMove(c, { clientX: barX(1) + 34 });
    await fireEvent.pointerUp(c);
    expect(moves).toEqual([]);
  });

  it('drag and keys are off while locked', async () => {
    const c = mount({ locked: true });
    await fireEvent.pointerDown(c, { clientX: barX(1) });
    await fireEvent.pointerMove(c, { clientX: barX(1) + 4 });
    await fireEvent.pointerUp(c);
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(moves).toEqual([]);
    expect(c.getAttribute('role')).toBe('img');
  });

  it('arrows move the selected change by absolute steps and stop at 256', async () => {
    const c = mount();
    await fireEvent.focus(c);
    // offset of change 1 is +8 ms = 1 step (8/7.8125)
    expect(c.getAttribute('aria-valuetext')).toBe('Tempo change 1 of 2 at 0:06.508, 90 BPM, +1/64 beat');
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(moves).toEqual([[1, 2]]);
    await fireEvent.keyDown(c, { key: 'ArrowRight', shiftKey: true });
    expect(moves[1]).toEqual([1, 66]);
    await fireEvent.keyDown(c, { key: 'ArrowLeft' });
    expect(moves[2]).toEqual([1, 65]);
    for (let i = 0; i < 4; i++) await fireEvent.keyDown(c, { key: 'ArrowRight', shiftKey: true });
    expect(moves[moves.length - 1]).toEqual([1, 256]);
    const n = moves.length;
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(moves.length).toBe(n);
  });

  it('up and down move the selection between detected changes only', async () => {
    const c = mount();
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Tempo change 2 of 2 at 0:15\.000, 110 BPM/);
    await fireEvent.keyDown(c, { key: 'ArrowLeft' });
    expect(moves[0][0]).toBe(3);
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Tempo change 2 of 2/);
    await fireEvent.keyDown(c, { key: 'ArrowUp' });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Tempo change 1 of 2/);
  });
});
