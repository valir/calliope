<script lang="ts">
  import * as Tabs from '$lib/components/ui/tabs/index.js';
  import { TRACK_TABS, VIEWS } from '$lib/views';
  const view = VIEWS.find((v) => v.id === 'editor')!;
</script>

<section class="flex flex-col gap-4 p-8" aria-labelledby="view-heading">
  <h1 id="view-heading" tabindex="-1" class="text-3xl font-semibold tracking-tight">{view.label}</h1>
  <p class="text-lg text-muted-foreground">{view.summary}</p>
  <Tabs.Root value="stems">
    <Tabs.List>
      {#each TRACK_TABS as tab (tab.id)}
        <Tabs.Trigger value={tab.id}>{tab.label}</Tabs.Trigger>
      {/each}
    </Tabs.List>
    {#each TRACK_TABS as tab (tab.id)}
      <Tabs.Content value={tab.id}>
        <p class="mt-4 rounded-lg border border-dashed border-border p-6 text-lg" data-testid="placeholder">
          This tab will be filled by: {tab.feature}.
        </p>
      </Tabs.Content>
    {/each}
  </Tabs.Root>
</section>
