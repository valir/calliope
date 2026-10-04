import { afterEach, describe, expect, it } from 'vitest';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { appVersion, frontendLog, getSettings, setTheme } from './ipc';

afterEach(() => clearMocks());

describe('ipc wrappers', () => {
  it('app_version', async () => {
    let seen: [string, unknown] | undefined;
    mockIPC((cmd, args) => { seen = [cmd, args]; return '1.2.3'; });
    expect(await appVersion()).toBe('1.2.3');
    expect(seen![0]).toBe('app_version');
  });

  it('get_settings', async () => {
    let cmd = '';
    mockIPC((c) => { cmd = c; return { theme: 'dark' }; });
    expect(await getSettings()).toEqual({ theme: 'dark' });
    expect(cmd).toBe('get_settings');
  });

  it('set_theme sends the theme argument', async () => {
    let seen: [string, unknown] | undefined;
    mockIPC((cmd, args) => { seen = [cmd, args]; return { theme: 'light' }; });
    expect(await setTheme('light')).toEqual({ theme: 'light' });
    expect(seen).toEqual(['set_theme', { theme: 'light' }]);
  });

  it('frontend_log sends the message', async () => {
    let seen: [string, unknown] | undefined;
    mockIPC((cmd, args) => { seen = [cmd, args]; });
    await frontendLog('hello');
    expect(seen).toEqual(['frontend_log', { message: 'hello' }]);
  });

  it('frontendLog swallows a rejected invoke', async () => {
    mockIPC(() => { throw new Error('boom'); });
    await expect(frontendLog('x')).resolves.toBeUndefined();
  });
});
