<script lang="ts">
  import * as RadioGroup from '$lib/components/ui/radio-group/index.js';
  import { Label } from '$lib/components/ui/label/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Progress } from '$lib/components/ui/progress/index.js';
  import {
    answerPrompt,
    browse,
    cancelRunning,
    chooseSource,
    extractUrl,
    imp,
    type SourceChoice,
  } from '$lib/import-state.svelte';
  import { formatMb } from '$lib/format';

  const job = $derived(imp.job);
  const downloading = $derived(job?.phase === 'downloading');
  const preparing = $derived(job?.phase === 'preparing');
  const locked = $derived(imp.starting || job !== null || imp.prompt !== null);

  const percent = $derived(
    job && job.total ? Math.min(100, (job.downloaded / job.total) * 100) : null,
  );
  const progressText = $derived(
    !job
      ? ''
      : job.total
        ? `${Math.round(percent ?? 0)}% (${formatMb(job.downloaded)} of ${formatMb(job.total)})`
        : formatMb(job.downloaded),
  );

  const options: { value: SourceChoice; label: string }[] = [
    { value: 'url', label: 'URL' },
    { value: 'audio', label: 'Local Audio File' },
    { value: 'video', label: 'Local Video File' },
  ];

  function onUrlKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      void extractUrl();
    }
  }
</script>

<section class="flex max-w-xl flex-col gap-4" aria-label="Select source">
  <h3 class="text-lg font-medium">Select source</h3>

  <RadioGroup.Root
    value={imp.source ?? ''}
    onValueChange={(v) => chooseSource(v as SourceChoice)}
    disabled={locked}
    aria-label="Source"
  >
    {#each options as o (o.value)}
      <div class="flex items-center gap-2">
        <RadioGroup.Item value={o.value} id="import-source-{o.value}" />
        <Label for="import-source-{o.value}" class="text-base">{o.label}</Label>
      </div>
    {/each}
  </RadioGroup.Root>

  {#if imp.source === 'url'}
    <div class="flex flex-col gap-2">
      <div class="flex gap-2">
        <Input
          id="import-url"
          type="text"
          aria-label="enter url"
          placeholder="enter url"
          autocomplete="off"
          spellcheck={false}
          disabled={locked}
          aria-describedby={imp.error ? 'import-error' : undefined}
          bind:value={imp.url}
          onkeydown={onUrlKey}
        />
        <Button disabled={locked} onclick={() => void extractUrl()}>Extract</Button>
      </div>
    </div>
  {:else if imp.source === 'audio' || imp.source === 'video'}
    <div>
      <Button disabled={locked} onclick={() => void browse(imp.source === 'audio' ? 'audio' : 'video')}>Browse</Button>
    </div>
  {/if}

  {#if imp.error}
    <p id="import-error" role="alert" class="text-sm text-primary">{imp.error}</p>
  {/if}

  {#if imp.prompt}
    <div role="group" aria-label="Incomplete download" class="flex flex-col gap-3 rounded-lg border border-primary/60 p-4">
      <p>Incomplete download file from the same URL found</p>
      <div class="flex gap-2">
        <Button onclick={() => void answerPrompt(true)}>Resume Download</Button>
        <Button variant="outline" onclick={() => void answerPrompt(false)}>Start Over</Button>
      </div>
    </div>
  {/if}

  {#if downloading}
    <div class="flex flex-col gap-2" role="group" aria-label="Download">
      <div class="flex items-baseline justify-between text-sm">
        <span id="import-download-label">Download in progress</span>
        <span class="text-muted-foreground" data-testid="download-text">{progressText}</span>
      </div>
      <Progress value={percent} aria-labelledby="import-download-label" />
      <div><Button variant="outline" size="sm" onclick={() => void cancelRunning()}>Cancel</Button></div>
    </div>
  {:else if preparing}
    <div class="flex flex-col gap-2" role="status">
      <span>Preparing audio...</span>
      <Progress value={null} aria-label="Preparing audio" />
      <div><Button variant="outline" size="sm" onclick={() => void cancelRunning()}>Cancel</Button></div>
    </div>
  {/if}
</section>
