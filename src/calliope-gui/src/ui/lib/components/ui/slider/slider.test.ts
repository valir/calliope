import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import Slider from './slider.svelte';
import Checkbox from '../checkbox/checkbox.svelte';

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

afterEach(cleanup);

describe('Slider', () => {
  it('arrow keys change the value by one step and the aria props are set', async () => {
    const { container } = render(Slider, {
      props: { value: 10, min: 0, max: 100, step: 5, 'aria-label': 'Gain', 'aria-valuetext': '10 dB' },
    });
    const thumb = container.querySelector('[data-slot="slider-thumb"]') as HTMLElement;
    expect(thumb.getAttribute('aria-label')).toBe('Gain');
    expect(thumb.getAttribute('aria-valuetext')).toBe('10 dB');
    expect(thumb.getAttribute('aria-valuenow')).toBe('10');
    thumb.focus();
    await fireEvent.keyDown(thumb, { key: 'ArrowRight' });
    expect(thumb.getAttribute('aria-valuenow')).toBe('15');
    await fireEvent.keyDown(thumb, { key: 'ArrowLeft' });
    await fireEvent.keyDown(thumb, { key: 'ArrowLeft' });
    expect(thumb.getAttribute('aria-valuenow')).toBe('5');
  });

  it('draws the tick at the right position, with no static style attribute in the markup', () => {
    const { container } = render(Slider, {
      props: { value: 0, min: -60, max: 12, tick: 0, 'aria-label': 'Gain' },
    });
    const tick = container.querySelector('[data-slot="slider-tick"]') as HTMLElement;
    expect(tick.style.left).toBe(`${(60 / 72) * 100}%`);
  });

  it('draws no tick when none is given', () => {
    const { container } = render(Slider, { props: { value: 1, 'aria-label': 'x' } });
    expect(container.querySelector('[data-slot="slider-tick"]')).toBeNull();
  });
});

describe('Checkbox', () => {
  it('Space toggles it', async () => {
    const { container } = render(Checkbox, { props: { 'aria-label': 'Enable' } });
    const box = container.querySelector('[data-slot="checkbox"]') as HTMLElement;
    expect(box.getAttribute('data-state')).toBe('unchecked');
    box.focus();
    await fireEvent.keyDown(box, { key: ' ' });
    await fireEvent.keyUp(box, { key: ' ' });
    expect(box.getAttribute('data-state')).toBe('checked');
    expect(box.getAttribute('aria-checked')).toBe('true');
  });
});
