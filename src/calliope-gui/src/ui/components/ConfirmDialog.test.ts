import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import ConfirmDialog from './ConfirmDialog.svelte';

afterEach(cleanup);

function setup() {
  const onanswer = vi.fn();
  render(ConfirmDialog, { open: true, message: 'Delete this track?', onanswer });
  return onanswer;
}

describe('ConfirmDialog', () => {
  it('renders the message', async () => {
    setup();
    expect(await screen.findByText('Delete this track?')).toBeTruthy();
  });

  it('Yes answers true', async () => {
    const onanswer = setup();
    await fireEvent.click(await screen.findByRole('button', { name: 'Yes' }));
    expect(onanswer).toHaveBeenCalledExactlyOnceWith(true);
  });

  it('No answers false', async () => {
    const onanswer = setup();
    await fireEvent.click(await screen.findByRole('button', { name: 'No' }));
    expect(onanswer).toHaveBeenCalledExactlyOnceWith(false);
  });

  it('initial focus is on No', async () => {
    setup();
    const no = await screen.findByRole('button', { name: 'No' });
    await waitFor(() => expect(document.activeElement).toBe(no));
  });

  it('Escape answers No', async () => {
    const onanswer = setup();
    const no = await screen.findByRole('button', { name: 'No' });
    await fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(onanswer).toHaveBeenCalledExactlyOnceWith(false));
  });
});
