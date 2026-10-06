import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import TrackFields from './TrackFields.svelte';
import type { DraftFields } from '$lib/track-draft';

afterEach(cleanup);

const draft: DraftFields = {
  band: 'Amber Fields',
  album: 'Copper Sky',
  title: 'Slow Burn',
  composers: 'Ann\nBo',
  year: '2019',
  source_url: 'https://example.org/x',
  copyright: '',
};
const input = (id: string) => document.getElementById(id) as HTMLInputElement;

describe('TrackFields', () => {
  it('edit mode: editable boxes with the import- prefix, errors linked by aria', async () => {
    const oninput = vi.fn();
    render(TrackFields, {
      draft,
      editing: true,
      errors: { year: 'Year must be a number', title: 'Title is required' },
      oninput,
      idPrefix: 'import',
    });
    for (const f of ['band', 'album', 'title', 'composers', 'year', 'source_url', 'copyright']) {
      expect(input(`import-${f}`)).toBeTruthy();
      expect(input(`import-${f}`).readOnly).toBe(false);
    }
    expect(screen.getByLabelText('Title')).toBe(input('import-title'));
    expect(input('import-title').value).toBe('Slow Burn');
    expect(input('import-composers').placeholder).toBe('One composer per line');
    expect(input('import-year').getAttribute('aria-describedby')).toBe('import-year-error');
    expect(document.getElementById('import-year-error')!.textContent).toBe('Year must be a number');
    expect(input('import-title').getAttribute('aria-invalid')).toBe('true');
    await fireEvent.input(input('import-band'), { target: { value: 'New' } });
    expect(oninput).toHaveBeenCalledWith('band', 'New');
  });

  it('view mode: read-only, no error text; no selection disables everything', () => {
    const { unmount } = render(TrackFields, {
      draft, editing: false, errors: {}, oninput: () => {}, idPrefix: 'import',
    });
    expect(input('import-band').readOnly).toBe(true);
    expect(input('import-band').disabled).toBe(false);
    expect(input('import-composers').readOnly).toBe(true);
    expect(document.getElementById('import-year-error')).toBeNull();
    unmount();
    render(TrackFields, { draft: null, editing: false, errors: {}, oninput: () => {}, idPrefix: 'track' });
    expect(input('track-band').disabled).toBe(true);
    expect(input('track-band').value).toBe('');
  });
});
