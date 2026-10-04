<script lang="ts">
  import { featureText, type ViewInfo } from '$lib/views';
  let { view }: { view: ViewInfo } = $props();
</script>

<section class="flex flex-col gap-4 p-8" aria-labelledby="view-heading">
  <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">{view.label}</h1>
  <p class="text-lg text-muted-foreground">{view.summary}</p>
  <p class="rounded-lg border border-dashed border-border p-6 text-lg" data-testid="placeholder">
    {featureText(view.features)}
  </p>
</section>
