<script lang="ts">
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Progress } from '$lib/components/ui/progress/index.js';
  import { cancelRunning, imp } from '$lib/import-state.svelte';
  import { formatMb } from '$lib/format';

  const job = $derived(imp.job);
  const phase = $derived(job?.phase);

  const sentPercent = $derived(job && job.total ? (job.sent / job.total) * 100 : null);
  const workPercent = $derived(job?.progress == null ? null : Math.round(job.progress * 100));
  const recvPercent = $derived(
    job && job.stems_total ? (job.stems_done / job.stems_total) * 100 : null,
  );
</script>

<section class="flex max-w-xl flex-col gap-4" aria-label="Stem extraction progress">
  <h3 class="text-lg font-medium">Extracting stems</h3>

  {#if phase === 'uploading'}
    <div class="flex flex-col gap-2" role="group" aria-label="Upload">
      <div class="flex items-baseline justify-between text-sm">
        <span id="extract-upload-label">Sending to the edge-AI server</span>
        {#if job && job.total}
          <span class="text-muted-foreground">{formatMb(job.sent)} of {formatMb(job.total)}</span>
        {/if}
      </div>
      <Progress value={sentPercent} aria-labelledby="extract-upload-label" />
    </div>
  {:else if phase === 'queued'}
    <div class="flex flex-col gap-2" role="status">
      <span>Waiting for the edge-AI server...</span>
      <Progress value={null} aria-label="Waiting for the edge-AI server" />
    </div>
  {:else if phase === 'working'}
    <div class="flex items-center gap-3" role="status">
      <LoaderCircle class="size-6 animate-spin text-primary" aria-hidden="true" />
      <span class="text-base">Working...</span>
      {#if workPercent !== null}<span class="text-base text-muted-foreground">{workPercent}%</span>{/if}
    </div>
  {:else if phase === 'receiving'}
    <div class="flex flex-col gap-2" role="group" aria-label="Receiving stems">
      <span id="extract-receive-label" class="text-sm">Receiving stems {job?.stems_done ?? 0} of {job?.stems_total ?? 0}</span>
      <Progress value={recvPercent} aria-labelledby="extract-receive-label" />
    </div>
  {:else if phase === 'saving'}
    <div class="flex items-center gap-3" role="status">
      <LoaderCircle class="size-5 animate-spin text-primary" aria-hidden="true" />
      <span>Saving the track</span>
    </div>
  {/if}

  {#if phase !== 'saving'}
    <div><Button variant="outline" onclick={() => void cancelRunning()}>Cancel</Button></div>
  {/if}
</section>
