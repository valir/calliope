import { afterEach, describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { cleanup, render } from '@testing-library/svelte';
import TrackView from '../views/TrackView.svelte';

const css = readFileSync(resolve(process.cwd(), 'src/ui/app.css'), 'utf8');

afterEach(cleanup);

describe('shadcn variants match bits-ui attributes', () => {
  it('bits-ui emits the attributes the variants map onto', () => {
    const { container } = render(TrackView);
    const root = container.querySelector('[data-slot="tabs"]')!;
    expect(root.getAttribute('data-orientation')).toBe('horizontal');
    expect(root.className).toContain('data-horizontal:flex-col');
    const active = container.querySelectorAll('[data-slot="tabs-trigger"][data-state="active"]');
    expect(active.length).toBe(1);
    expect(active[0].className).toContain('data-active:bg-background');
  });

  it('app.css maps the Base UI style variants to bits-ui attributes', () => {
    for (const [name, attr] of [
      ['data-horizontal', 'data-orientation="horizontal"'],
      ['data-vertical', 'data-orientation="vertical"'],
      ['data-active', 'data-state="active"'],
      ['data-open', 'data-state="open"'],
      ['data-closed', 'data-state="closed"'],
      ['data-checked', 'data-state="checked"'],
      ['data-unchecked', 'data-state="unchecked"'],
    ]) {
      expect(css).toContain(`@custom-variant ${name} (&[${attr}]);`);
    }
  });
});
