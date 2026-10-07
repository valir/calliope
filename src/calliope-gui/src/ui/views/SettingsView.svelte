<script lang="ts">
  import * as Card from '$lib/components/ui/card/index.js';
  import * as RadioGroup from '$lib/components/ui/radio-group/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import RepositorySettings from '../components/RepositorySettings.svelte';
  import StemSettings from '../components/StemSettings.svelte';
  import ToolsSettings from '../components/ToolsSettings.svelte';
  import { ui } from '$lib/app-state.svelte';
  import { frontendLog, setTheme, type Theme } from '$lib/ipc';
  import { applyTheme } from '$lib/theme';
  import { SETTINGS_PLACEHOLDERS, VIEWS } from '$lib/views';

  const view = VIEWS.find((v) => v.id === 'settings')!;

  async function onThemeChange(value: string): Promise<void> {
    if (value !== 'dark' && value !== 'light') return;
    const theme: Theme = value;
    applyTheme(theme);
    ui.theme = theme;
    void frontendLog(`theme=${theme}`);
    try {
      await setTheme(theme);
    } catch (err) {
      void frontendLog(`error set_theme: ${String(err)}`);
    }
  }
</script>

<section class="flex flex-col gap-4 p-8" aria-labelledby="view-heading">
  <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">{view.label}</h1>
  <p class="text-lg text-muted-foreground">{view.summary}</p>

  <Card.Root>
    <Card.Header>
      <Card.Title>Appearance</Card.Title>
      <Card.Description>Colour theme of the application.</Card.Description>
    </Card.Header>
    <Card.Content>
      <RadioGroup.Root
        aria-label="Theme"
        value={ui.theme}
        onValueChange={onThemeChange}
        class="flex gap-6"
      >
        <div class="flex items-center gap-2">
          <RadioGroup.Item value="dark" id="theme-dark" />
          <Label for="theme-dark">Dark</Label>
        </div>
        <div class="flex items-center gap-2">
          <RadioGroup.Item value="light" id="theme-light" />
          <Label for="theme-light">Light</Label>
        </div>
      </RadioGroup.Root>
    </Card.Content>
  </Card.Root>

  <RepositorySettings />

  <StemSettings />

  <ToolsSettings />

  {#each SETTINGS_PLACEHOLDERS as s (s.id)}
    <Card.Root>
      <Card.Header>
        <Card.Title>{s.title}</Card.Title>
        <Card.Description>{s.text}</Card.Description>
      </Card.Header>
      <Card.Content class="flex flex-col gap-3">
        <p class="text-muted-foreground">This section will be filled by: {s.feature}.</p>
      </Card.Content>
    </Card.Root>
  {/each}
</section>
