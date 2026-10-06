<script lang="ts">
  import { onMount } from 'svelte';
  import * as Card from '$lib/components/ui/card/index.js';
  import { checkTools, type ToolInfo } from '$lib/ipc';

  let rows = $state<{ name: string; info: ToolInfo }[]>([]);
  let error = $state('');

  onMount(async () => {
    try {
      const t = await checkTools();
      rows = [
        { name: 'yt-dlp', info: t.yt_dlp },
        { name: 'ffmpeg', info: t.ffmpeg },
        { name: 'ffprobe', info: t.ffprobe },
      ];
    } catch (err) {
      error = `Could not check the tools: ${String(err)}`;
    }
  });

  function line(i: ToolInfo): string {
    if (!i.found) return i.message;
    const v = i.version ? `version ${i.version}` : 'found';
    return i.ok ? v : `${v}. ${i.message}`;
  }
</script>

<Card.Root>
  <Card.Header>
    <Card.Title>External tools</Card.Title>
    <Card.Description>Programs Calliope uses to download and prepare audio.</Card.Description>
  </Card.Header>
  <Card.Content class="flex flex-col gap-2">
    {#if error}<p role="alert" class="text-primary">{error}</p>{/if}
    {#each rows as r (r.name)}
      <p data-testid="tool-{r.name}">
        <span class="font-mono">{r.name}</span>:
        <span class={r.info.ok ? 'text-muted-foreground' : 'text-primary'}>{line(r.info)}</span>
      </p>
    {/each}
  </Card.Content>
</Card.Root>
