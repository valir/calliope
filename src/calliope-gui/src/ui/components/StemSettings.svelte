<script lang="ts">
  import { onMount, tick } from 'svelte';
  import * as Card from '$lib/components/ui/card/index.js';
  import * as Switch from '$lib/components/ui/switch/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import { applyEdgeAi, ui } from '$lib/app-state.svelte';
  import {
    checkEdgeAi,
    frontendLog,
    getSettings,
    setEdgeAiUrl,
    setKeepOriginal,
    type EdgeAiStatus,
  } from '$lib/ipc';

  let card = $state<HTMLElement | null>(null);
  let address = $state('');
  let saved = $state('');
  let keep = $state(false);
  let error = $state('');
  let status = $state<EdgeAiStatus | null>(null);
  let busy = $state(false);

  async function setStatus(st: EdgeAiStatus): Promise<void> {
    status = st;
    applyEdgeAi(st.state);
    void frontendLog(`settings edge-ai state=${st.state}`);
  }

  async function test(): Promise<void> {
    status = null;
    try {
      await setStatus(await checkEdgeAi());
    } catch (err) {
      error = `Could not test the connection: ${String(err)}`;
    }
  }

  async function save(): Promise<void> {
    if (busy) return;
    busy = true;
    error = '';
    try {
      const s = await setEdgeAiUrl(address.trim() === '' ? null : address.trim());
      saved = s.edge_ai_url ?? '';
      address = saved;
      void frontendLog(`settings edge_ai_url=${saved === '' ? 'none' : saved}`);
      await test();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }

  async function onKeepChange(next: boolean): Promise<void> {
    error = '';
    try {
      keep = (await setKeepOriginal(next)).keep_original;
      void frontendLog(`settings keep_original=${keep}`);
    } catch (err) {
      keep = !next;
      error = `Could not save the setting: ${String(err)}`;
    }
  }

  onMount(async () => {
    try {
      const s = await getSettings();
      saved = s.edge_ai_url ?? '';
      if (address === '') address = saved;
      keep = s.keep_original;
    } catch (err) {
      error = `Could not read the settings: ${String(err)}`;
    }
    if (ui.settingsTarget === 'stem') {
      ui.settingsTarget = '';
      await tick();
      card?.scrollIntoView?.({ block: 'center' });
      card?.querySelector<HTMLElement>('#edge-ai-address')?.focus();
    }
  });
</script>

<Card.Root bind:ref={card} data-testid="stem-settings">
  <Card.Header>
    <Card.Title>Stem extraction</Card.Title>
    <Card.Description>The edge-AI server on your network that separates a song into stems.</Card.Description>
  </Card.Header>
  <Card.Content class="flex flex-col gap-4">
    <div class="flex flex-col gap-2">
      <Label for="edge-ai-address">Edge-AI server address</Label>
      <div class="flex gap-3">
        <Input
          id="edge-ai-address"
          placeholder="http://archserver:8765"
          bind:value={address}
          onkeydown={(e) => e.key === 'Enter' && void save()}
        />
        <Button variant="outline" disabled={busy} onclick={save}>Save</Button>
        <Button variant="outline" disabled={busy || saved === ''} onclick={test}>Test connection</Button>
      </div>
      {#if error}
        <p role="alert" class="text-amber-700 dark:text-primary">{error}</p>
      {/if}
      {#if status}
        <p
          data-testid="edge-ai-status"
          class={status.state === 'connected' || status.state === 'not-configured' ? 'text-muted-foreground' : 'text-amber-700 dark:text-primary'}
        >
          {status.message}
        </p>
      {/if}
    </div>
    <div class="flex items-center gap-3">
      <Switch.Root id="keep-original" checked={keep} onCheckedChange={onKeepChange} />
      <Label for="keep-original">Keep the original mix with the stems</Label>
    </div>
  </Card.Content>
</Card.Root>
