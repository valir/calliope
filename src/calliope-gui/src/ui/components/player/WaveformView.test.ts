import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import WaveformView from './WaveformView.svelte';
import { columns, fineMs, loopMarks, sectionMarks, xOf } from '$lib/waveform-geometry';
import type { PlayerLoop, TempoSegment } from '$lib/ipc';

const mk = (id: string, start_ms: number, end_ms: number | null): PlayerLoop => ({
  id, start: { ms: start_ms }, end: end_ms === null ? null : { ms: end_ms }, start_ms, end_ms, repeat: false, count_in: false,
  training: { on: false, start: 70, end: 100, step: 5, repeats: 2 },
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
    expect(m0.x).toBe(0);
    expect(bars().map((r) => r.x)).toEqual([0, m1.x, m2.x]);
    expect(texts).toEqual([
      { t: '120 BPM', x: 5, y: 92 },
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
    // the first bar is movable whenever a tempo exists; without a tempo nothing is
    const typed = mount({ segments: [seg(0, 120), seg(6500, 90)] });
    expect(typed.getAttribute('role')).toBe('slider');
    cleanup();
    expect(mount({ segments: [seg(0, 0)] }).getAttribute('role')).toBe('img');
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

  it('resets the drag instead of throwing when the segments shrink mid-drag', async () => {
    const onmove = (i: number, s: number) => moves.push([i, s]);
    moves = [];
    const { rerender } = render(WaveformView, { props: { ...base, segments: edit, onmove } });
    const c = screen.getByTestId('wf');
    await fireEvent.pointerDown(c, { clientX: barX(3) });
    await fireEvent.pointerMove(c, { clientX: barX(3) + 4 });
    expect(screen.queryByTestId('wf-readout')).not.toBeNull();
    await rerender({ ...base, segments: edit.slice(0, 2), onmove });
    await tick();
    expect(screen.queryByTestId('wf-readout')).toBeNull();
    await fireEvent.pointerUp(c);
    expect(moves).toEqual([]);
  });

  it('changing the selected bar with Up / Down clears the keyboard readout', async () => {
    const c = mount();
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(screen.queryByTestId('wf-readout')).not.toBeNull();
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
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
    await fireEvent.keyDown(c, { key: 'ArrowDown' }); // the first bar is selected first
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
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Tempo change 2 of 2 at 0:15\.000, 110 BPM/);
    await fireEvent.keyDown(c, { key: 'ArrowLeft' });
    expect(moves[0][0]).toBe(3);
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Tempo change 2 of 2/);
    await fireEvent.keyDown(c, { key: 'ArrowUp' });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Tempo change 1 of 2/);
    await fireEvent.keyDown(c, { key: 'ArrowUp' });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Section 1 of 4 from the start, 120 BPM/);
  });

  it('the first bar is selectable and movable in the Editor mode', async () => {
    const found = [{ ...seg(1200, 120), found_ms: 1200 }, ...edit.slice(1)];
    const c = mount({ segments: found });
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(moves).toEqual([[0, 1]]);
    cleanup();
    const moved = mount({ segments: [{ ...seg(1207.8125, 120), found_ms: 1200 }, ...edit.slice(1)] });
    await fireEvent.focus(moved);
    expect(moved.getAttribute('aria-valuetext')).toBe('Section 1 of 4 from the start, 120 BPM, first beat, +1/64 beat');
  });
});

describe('WaveformView first bar', () => {
  const found = (start: number | null): TempoSegment[] => [
    { start_ms: start ?? 1200, bpm: 120, ...(start === null ? {} : { found_ms: 1200 }) },
  ];
  let moves: [number, number][];
  const mountF = (segments: TempoSegment[]) => {
    moves = [];
    render(WaveformView, { props: { ...base, segments, freeMove: true, selected: 0, onselect() {}, onmove: (i: number, s: number) => moves.push([i, s]) } });
    return screen.getByTestId('wf');
  };
  it('draws the first bar at 1200 ms; a drag by one fine step shows +1/64 beat and sends onmove(0, 1)', async () => {
    cssWidth = 2500; // 1 px = 8.0 ms
    const c = mountF([{ start_ms: 1200, bpm: 120, found_ms: 1200 }]);
    await tick();
    const x = xOf(1200, 20000, 2500);
    expect(rects.some((r) => r.h === 96 && r.x === x - 1 && r.w === 4)).toBe(true); // selected: the 4 px bar
    await fireEvent.pointerDown(c, { clientX: x });
    await fireEvent.pointerMove(c, { clientX: x + 1 });
    expect(screen.getByTestId('wf-readout').textContent).toContain('+1/64 beat');
    await fireEvent.pointerUp(c);
    expect(moves).toEqual([[0, 1]]);
  });

  it('Shift+Left stops at 0:00 and Right stops at the end limit', async () => {
    const c = mountF(found(1200));
    await fireEvent.focus(c);
    for (let i = 0; i < 40; i++) await fireEvent.keyDown(c, { key: 'ArrowLeft', shiftKey: true });
    expect(moves[moves.length - 1]).toEqual([0, -154]);
    cleanup();
    const d = mountF(found(1200));
    await fireEvent.focus(d);
    for (let i = 0; i < 600; i++) await fireEvent.keyDown(d, { key: 'ArrowRight', shiftKey: true });
    // hi = 20000 - 500 ms
    expect(moves[moves.length - 1][1]).toBe(Math.ceil((19500 - 1200) / fineMs(120)));
  });

  it('dims the lead-in only when the first beat is set', async () => {
    mountF(found(1200));
    await tick();
    expect(rects.some((r) => r.x === 0 && r.w === xOf(1200, 20000, 200) && r.h === 96)).toBe(true);
    cleanup();
    mountF(found(null));
    await tick();
    expect(rects.some((r) => r.x === 0 && r.w === xOf(1200, 20000, 200) && r.h === 96)).toBe(false);
  });

  it('draws the dim with the background colour at 0.6 alpha', async () => {
    const alphas: [number, string][] = [];
    let alpha = 1;
    let fill = '';
    (HTMLCanvasElement.prototype.getContext as unknown as { mockImplementation: (f: unknown) => void }).mockImplementation(() => ({
      setTransform() {}, clearRect() {}, measureText: () => ({ width: 10 }), fillText() {},
      set globalAlpha(v: number) { alpha = v; }, get globalAlpha() { return alpha; },
      set fillStyle(v: string) { fill = v; },
      fillRect: (x: number, _y: number, w: number) => { if (x === 0 && w === xOf(1200, 20000, 200)) alphas.push([alpha, fill]); },
    }));
    mountF(found(1200));
    await tick();
    expect(alphas).toContainEqual([0.6, '#fff']);
  });
});

describe('WaveformView Player mode (freeMove)', () => {
  // detected at 6500, placed (on_beat) at 10000, typed at 15000
  const ps: TempoSegment[] = [seg(0, 120), seg(6508, 90, 6500), { start_ms: 10000, bpm: 100, on_beat: true }, seg(15000, 110)];
  let calls: Record<string, unknown[][]>;
  const mountP = (extra: Record<string, unknown> = {}) => {
    calls = { move: [], select: [], label: [], remove: [], add: [], commit: [], invalid: [], cancel: [] };
    render(WaveformView, {
      props: {
        ...base, segments: ps, freeMove: true, selected: 0,
        onmove: (...a: unknown[]) => calls.move.push(a), onselect: (...a: unknown[]) => calls.select.push(a),
        onlabel: (...a: unknown[]) => calls.label.push(a), onremove: (...a: unknown[]) => calls.remove.push(a),
        onadd: (...a: unknown[]) => calls.add.push(a), oncommitbpm: (...a: unknown[]) => calls.commit.push(a),
        oninvalidbpm: (...a: unknown[]) => calls.invalid.push(a), oncancelbpm: (...a: unknown[]) => calls.cancel.push(a),
        ...extra,
      },
    });
    return screen.getByTestId('wf');
  };
  const px = (i: number) => sectionMarks(ps, 20000, 200)[i].x!;
  const pxMs = 20000 / 199;

  it('marks the canvas data-player-keys; the Editor mode does not', () => {
    expect(mountP().hasAttribute('data-player-keys')).toBe(true);
    cleanup();
    render(WaveformView, { props: { ...base, segments: ps, onmove() {} } });
    expect(screen.getByTestId('wf').hasAttribute('data-player-keys')).toBe(false);
  });

  it('a drag of a placed change sends one onmove measured from the left start, without the 256 limit', async () => {
    const c = mountP();
    const x0 = px(2);
    await fireEvent.pointerDown(c, { clientX: x0, clientY: 10 });
    await fireEvent.pointerMove(c, { clientX: x0 + 30 });
    expect(screen.getByTestId('wf-readout').textContent).toMatch(/^[+-]\d+\/64 beat \([+-]0:\d\d\.\d{3}\)$/);
    await fireEvent.pointerUp(c);
    expect(calls.move.length).toBe(1);
    const expected = Math.round(((x0 + 30) * pxMs - 6508) / fineMs(90));
    expect(calls.move[0][0]).toBe(2);
    expect(Math.abs((calls.move[0][1] as number) - expected)).toBeLessThanOrEqual(1);
    expect(calls.move[0][1] as number).toBeGreaterThan(256); // past the detected limit
    expect(calls.select).toEqual([[2]]);
  });

  it('a detected change still stops at 256; Escape cancels', async () => {
    const c = mountP();
    await fireEvent.pointerDown(c, { clientX: px(1), clientY: 10 });
    await fireEvent.pointerMove(c, { clientX: px(1) + 60 });
    expect(screen.getByTestId('wf-readout').textContent).toMatch(/limit ±4 beats$/);
    await fireEvent.pointerUp(c);
    expect(calls.move[0]).toEqual([1, 256]);
    calls.move.length = 0;
    await fireEvent.pointerDown(c, { clientX: px(1), clientY: 10 });
    await fireEvent.pointerMove(c, { clientX: px(1) + 4 });
    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.pointerUp(c);
    expect(calls.move).toEqual([]);
  });

  it('a typed change can be dragged too', async () => {
    const c = mountP();
    await fireEvent.pointerDown(c, { clientX: px(3), clientY: 10 });
    await fireEvent.pointerMove(c, { clientX: px(3) - 12 });
    await fireEvent.pointerUp(c);
    expect(calls.move.length).toBe(1);
    expect(calls.move[0][0]).toBe(3);
  });

  it('a click on a label calls onlabel; a click elsewhere selects the section under the pointer', async () => {
    const c = mountP();
    await tick();
    await fireEvent.pointerDown(c, { clientX: px(1) + 12, clientY: 90 });
    await fireEvent.pointerUp(c);
    expect(calls.label).toEqual([[1]]);
    await fireEvent.pointerDown(c, { clientX: 20, clientY: 30 });
    await fireEvent.pointerDown(c, { clientX: px(2) + 40, clientY: 30 });
    await fireEvent.pointerUp(c);
    expect(calls.label.length).toBe(1);
    expect(calls.select).toEqual([[0], [2]]);
  });

  it('draws the target with a 4 px bar and an underlined label', async () => {
    mountP({ selected: 1 });
    await tick();
    expect(rects.filter((r) => r.w === 4 && r.h === 96).map((r) => r.x)).toEqual([px(1) - 1]);
    expect(rects.filter((r) => r.h === 2 && r.y === 94).length).toBe(1);
  });

  it('keys: arrows select (section 0 included) and move, Enter, Delete, a; Space is not prevented', async () => {
    const c = mountP({ selected: 1 });
    await fireEvent.focus(c);
    expect(c.getAttribute('aria-valuetext')).toBe('Section 2 of 4 from 0:06.508, 90 BPM, detected, +1/64 beat');
    await fireEvent.keyDown(c, { key: 'ArrowUp' });
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
    expect(calls.select).toEqual([[0], [2]]);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    await fireEvent.keyDown(c, { key: 'ArrowLeft', shiftKey: true });
    expect(calls.move).toEqual([[1, 2], [1, -62]]);
    await fireEvent.keyDown(c, { key: 'Enter' });
    await fireEvent.keyDown(c, { key: 'Delete' });
    await fireEvent.keyDown(c, { key: 'Backspace' });
    await fireEvent.keyDown(c, { key: 'a' });
    expect(calls.label).toEqual([[1]]);
    expect(calls.remove).toEqual([[1], [1]]);
    expect(calls.add.length).toBe(1);
    expect(await fireEvent.keyDown(c, { key: ' ' })).toBe(true);
    expect(await fireEvent.keyDown(c, { key: 'b' })).toBe(true);
  });

  it('reads out a placed or typed section', async () => {
    const c = mountP({ selected: 2 });
    expect(c.getAttribute('aria-valuetext')).toMatch(/^Section 3 of 4 from 0:10\.000, 100 BPM, placed, [+-]\d+\/64 beat$/);
    cleanup();
    expect(mountP({ selected: 3 }).getAttribute('aria-valuetext')).toBe('Section 4 of 4 from 0:15.000, 110 BPM, typed');
  });

  it('is read-only at BPM 0', async () => {
    const c = mountP({ segments: [seg(0, 0)] });
    expect(c.getAttribute('role')).toBe('img');
    await fireEvent.pointerDown(c, { clientX: 20, clientY: 30 });
    expect(calls.select).toEqual([]);
  });

  describe('BPM overlay', () => {
    const open = () => mountP({ bpmEdit: { index: 1 } });
    const input = () => screen.getByTestId('section-bpm-input') as HTMLInputElement;

    it('is prefilled and focused; Enter with 186 commits', async () => {
      open();
      await tick();
      expect(input().value).toBe('90');
      expect(document.activeElement).toBe(input());
      await fireEvent.input(input(), { target: { value: '186' } });
      await fireEvent.keyDown(input(), { key: 'Enter' });
      expect(calls.commit).toEqual([[186]]);
    });

    it('refuses 0 and 301 and stays open', async () => {
      open();
      for (const v of ['0', '301']) {
        await fireEvent.input(input(), { target: { value: v } });
        await fireEvent.keyDown(input(), { key: 'Enter' });
      }
      expect(calls.invalid).toEqual([['Enter a BPM between 20 and 300'], ['Enter a BPM between 20 and 300']]);
      expect(calls.commit).toEqual([]);
      expect(calls.cancel).toEqual([]);
      expect(screen.queryByTestId('section-bpm-input')).not.toBeNull();
    });

    it('Escape and blur cancel', async () => {
      open();
      await fireEvent.keyDown(input(), { key: 'Escape' });
      expect(calls.cancel.length).toBe(1);
      cleanup();
      open();
      await fireEvent.blur(input());
      expect(calls.cancel.length).toBe(1);
    });

    it('for Add it is prefilled with the BPM at the playhead', async () => {
      mountP({ bpmEdit: { addAt: 12000 } });
      expect(input().value).toBe('100');
    });
  });
});

describe('WaveformView Editor mode is unchanged by the Player props', () => {
  it('placed and typed changes do not start a drag without freeMove', async () => {
    const s: TempoSegment[] = [seg(0, 120), { start_ms: 10000, bpm: 100, on_beat: true }, seg(15000, 110, 15000)];
    const moves: unknown[] = [];
    render(WaveformView, { props: { ...base, segments: s, onmove: (...a: unknown[]) => moves.push(a) } });
    const c = screen.getByTestId('wf');
    const x = sectionMarks(s, 20000, 200)[1].x!;
    await fireEvent.pointerDown(c, { clientX: x });
    await fireEvent.pointerMove(c, { clientX: x + 6 });
    expect(screen.queryByTestId('wf-readout')).toBeNull();
    await fireEvent.pointerUp(c);
    expect(moves).toEqual([]);
  });
});

describe('WaveformView keyboard readout', () => {
  let moves: [number, number][];
  const mountK = (segments: TempoSegment[], extra: Record<string, unknown> = {}) => {
    moves = [];
    render(WaveformView, { props: { ...base, segments, onmove: (i: number, s: number) => moves.push([i, s]), ...extra } });
    return screen.getByTestId('wf');
  };
  const readout = () => screen.queryByTestId('wf-readout')?.textContent ?? null;
  const first = (start = 1200): TempoSegment[] => [{ start_ms: start, bpm: 120, found_ms: 1200 }];
  afterEach(() => vi.useRealTimers());

  it('Player mode: shows for 1.5 s after the last key, is refreshed by each key and goes on blur', async () => {
    vi.useFakeTimers();
    const c = mountK(first(), { freeMove: true, selected: 0, onselect() {} });
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(readout()).toBe('+1/64 beat (+0:00.008) from the found start');
    await vi.advanceTimersByTimeAsync(1499);
    expect(readout()).not.toBeNull();
    await vi.advanceTimersByTimeAsync(1);
    expect(readout()).toBeNull();

    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    await vi.advanceTimersByTimeAsync(1000);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(readout()).toBe('+3/64 beat (+0:00.023) from the found start');
    await vi.advanceTimersByTimeAsync(1400);
    expect(readout()).not.toBeNull();
    await vi.advanceTimersByTimeAsync(100);
    expect(readout()).toBeNull();

    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(readout()).not.toBeNull();
    await fireEvent.blur(c);
    expect(readout()).toBeNull();
  });

  it('a second key shows the next step count', async () => {
    vi.useFakeTimers();
    const c = mountK(first(), { freeMove: true, selected: 0, onselect() {} });
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    await vi.advanceTimersByTimeAsync(1000);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(readout()).toBe('+2/64 beat (+0:00.016) from the found start');
  });

  it('Editor mode: a detected change shows the same text as its drag, with the limit tail', async () => {
    vi.useFakeTimers();
    cssWidth = 2500; // 1 px = 8 ms
    const segs: TempoSegment[] = [seg(0, 120), seg(6500, 90, 6500)];
    const c = mountK(segs);
    const x = sectionMarks(segs, 20000, 2500)[1].x!;
    await fireEvent.pointerDown(c, { clientX: x });
    await fireEvent.pointerMove(c, { clientX: x + 1 });
    const dragText = readout();
    await fireEvent.pointerUp(c);
    expect(dragText).toMatch(/^\+1\/64 beat \(\+0:00\.008\) · limit ±4 beats$/);
    expect(readout()).toBeNull();
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowDown' });
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(readout()).toBe(dragText);
  });

  it('at the limit the readout shows and no move is sent', async () => {
    vi.useFakeTimers();
    const c = mountK(first(0), { freeMove: true, selected: 0, onselect() {} });
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowLeft' });
    expect(readout()).toMatch(/^-154\/64 beat .* from the found start$/);
    expect(moves).toEqual([]);
  });

  it('a pointerdown replaces the key readout by the drag readout, gone after pointerup', async () => {
    vi.useFakeTimers();
    cssWidth = 2500;
    const c = mountK(first(), { freeMove: true, selected: 0, onselect() {} });
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowRight' });
    expect(readout()).not.toBeNull();
    const x = xOf(1200, 20000, 2500);
    await fireEvent.pointerDown(c, { clientX: x, clientY: 10 });
    expect(readout()).toBe('0/64 beat (+0:00.000) from the found start');
    await fireEvent.pointerUp(c);
    expect(readout()).toBeNull();
  });
  it('Player mode: a typed section shows the offset from the previous start, a detected one the limit tail', async () => {
    vi.useFakeTimers();
    const segs: TempoSegment[] = [seg(1200, 120), seg(3000, 120, 3000), seg(4200, 120)];
    segs[0].found_ms = 1200;
    const c = mountK(segs, { freeMove: true, selected: 2, onselect() {} });
    await fireEvent.focus(c);
    await fireEvent.keyDown(c, { key: 'ArrowLeft' });
    expect(readout()).toBe('+153/64 beat (+0:01.195)');
    expect(moves).toEqual([[2, 153]]);
    await vi.advanceTimersByTimeAsync(1500);
    expect(readout()).toBeNull();
    cleanup();
    const d = mountK(segs, { freeMove: true, selected: 1, onselect() {} });
    await fireEvent.focus(d);
    await fireEvent.keyDown(d, { key: 'ArrowRight', shiftKey: true });
    expect(readout()).toMatch(/^\+64\/64 beat \(\+0:00\.500\) · limit ±\d+ beats$/);
  });
});
