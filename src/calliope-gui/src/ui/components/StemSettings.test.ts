import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import StemSettings from './StemSettings.svelte';
import ToolsSettings from './ToolsSettings.svelte';
import StatusFooter from './StatusFooter.svelte';
import { ui } from '$lib/app-state.svelte';
import type { EdgeAiStatus, ToolInfo } from '$lib/ipc';

let calls: { cmd: string; args: unknown }[] = [];
let settings = { theme: 'dark', repository_root: null as string | null, edge_ai_url: null as string | null, keep_original: false };
let health: EdgeAiStatus;
let tools: Record<string, ToolInfo>;

beforeEach(() => {
  calls = [];
  settings = { theme: 'dark', repository_root: null, edge_ai_url: null, keep_original: false };
  health = { state: 'not-configured', message: 'Not configured', models: [] };
  ui.edgeAi = 'not-configured';
  tools = {
    yt_dlp: { found: true, version: '2024.12.13', ok: true, message: '' },
    ffmpeg: { found: true, version: '7.1', ok: true, message: '' },
    ffprobe: { found: true, version: '7.1', ok: true, message: '' },
  };
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    const a = args as Record<string, unknown>;
    if (cmd === 'get_settings') return settings;
    if (cmd === 'set_edge_ai_url') {
      const u = a.url as string | null;
      if (u && !u.startsWith('http://')) throw 'The server address must start with http://';
      settings = { ...settings, edge_ai_url: u };
      return settings;
    }
    if (cmd === 'set_keep_original') {
      settings = { ...settings, keep_original: a.keep as boolean };
      return settings;
    }
    if (cmd === 'check_edge_ai') return health;
    if (cmd === 'check_tools') return tools;
    if (cmd === 'app_version') return '0.0.0';
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  clearMocks();
});

describe('StemSettings', () => {
  it('shows the saved address and the switch state from get_settings', async () => {
    settings.edge_ai_url = 'http://a:1';
    settings.keep_original = true;
    render(StemSettings);
    const box = (await screen.findByLabelText('Edge-AI server address')) as HTMLInputElement;
    await waitFor(() => expect(box.value).toBe('http://a:1'));
    expect(screen.getByRole('switch').getAttribute('aria-checked')).toBe('true');
  });

  it('an invalid address shows the Rust error and does not test', async () => {
    render(StemSettings);
    const box = await screen.findByLabelText('Edge-AI server address');
    await waitFor(() => expect(calls.map((c) => c.cmd)).toContain('get_settings'));
    await fireEvent.input(box, { target: { value: 'ftp://x' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect((await screen.findByRole('alert')).textContent).toContain('must start with http://');
    expect(calls.map((c) => c.cmd)).not.toContain('check_edge_ai');
  });

  it('Save then tests and shows Connected, updating the footer state', async () => {
    health = { state: 'connected', message: 'Connected (htdemucs_6s)', models: ['htdemucs_6s'] };
    render(StemSettings);
    const box = await screen.findByLabelText('Edge-AI server address');
    await waitFor(() => expect(calls.map((c) => c.cmd)).toContain('get_settings'));
    await fireEvent.input(box, { target: { value: 'http://archserver:8765' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect((await screen.findByTestId('edge-ai-status')).textContent).toContain('Connected (htdemucs_6s)');
    expect(calls.find((c) => c.cmd === 'set_edge_ai_url')!.args).toEqual({ url: 'http://archserver:8765' });
    expect(ui.edgeAi).toBe('connected');
  });

  it('Test connection shows the unreachable message', async () => {
    settings.edge_ai_url = 'http://a:1';
    health = { state: 'unreachable', message: 'The edge-AI server could not be reached.', models: [] };
    render(StemSettings);
    await waitFor(() => expect((screen.getByLabelText('Edge-AI server address') as HTMLInputElement).value).toBe('http://a:1'));
    await fireEvent.click(screen.getByRole('button', { name: 'Test connection' }));
    expect((await screen.findByTestId('edge-ai-status')).textContent).toContain('could not be reached');
    expect(ui.edgeAi).toBe('unreachable');
  });

  it('the switch calls set_keep_original', async () => {
    render(StemSettings);
    const sw = await screen.findByRole('switch');
    await fireEvent.click(sw);
    await waitFor(() => expect(calls.find((c) => c.cmd === 'set_keep_original')?.args).toEqual({ keep: true }));
    await waitFor(() => expect(sw.getAttribute('aria-checked')).toBe('true'));
  });

  it('"Change in Settings" target focuses the address box', async () => {
    ui.settingsTarget = 'stem';
    render(StemSettings);
    await waitFor(() => expect(document.activeElement?.id).toBe('edge-ai-address'));
    expect(ui.settingsTarget).toBe('');
  });
});

describe('StatusFooter', () => {
  it('shows the checked Edge-AI state', async () => {
    health = { state: 'connected', message: 'Connected', models: [] };
    render(StatusFooter);
    await waitFor(() => expect(screen.getByText(/Edge-AI: connected/)).toBeTruthy());
  });
});

describe('ToolsSettings', () => {
  it('shows versions and a missing tool install hint, no paths', async () => {
    tools.yt_dlp = { found: false, version: null, ok: false, message: 'yt-dlp was not found. Install it (Arch: sudo pacman -S yt-dlp) and try again.' };
    render(ToolsSettings);
    expect((await screen.findByTestId('tool-yt-dlp')).textContent).toContain('sudo pacman -S yt-dlp');
    expect(screen.getByTestId('tool-ffmpeg').textContent).toContain('version 7.1');
    expect(screen.getByTestId('tool-ffprobe').textContent).toContain('7.1');
  });
});
