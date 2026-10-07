import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';
import { frontendLog, getSettings } from '$lib/ipc';
import { applyTheme } from '$lib/theme';
import { ui } from '$lib/app-state.svelte';

// Registered first so that any violation during startup is reported.
document.addEventListener('securitypolicyviolation', (e) => {
  void frontendLog(`csp-violation directive=${e.violatedDirective} blocked=${e.blockedURI}`);
});

async function start(): Promise<void> {
  try {
    const settings = await getSettings();
    ui.theme = settings.theme === 'light' ? 'light' : 'dark';
  } catch (err) {
    ui.theme = 'dark';
    void frontendLog(`error get_settings: ${String(err)}`);
  }
  applyTheme(ui.theme);
  mount(App, { target: document.getElementById('app')! });
}

void start();
