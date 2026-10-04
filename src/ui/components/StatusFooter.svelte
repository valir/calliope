<script lang="ts">
  import AboutDialog from './AboutDialog.svelte';
  import { Button } from '$lib/components/ui/button/index.js';
  import { ui } from '$lib/app-state.svelte';

  let aboutOpen = $state(false);
</script>

<footer
  class="col-span-2 flex items-center justify-between border-t border-border bg-sidebar px-4 py-1 text-sm text-muted-foreground"
>
  <span>Ready · Edge-AI: not configured</span>
  <Button variant="ghost" size="sm" aria-label="About calliope-gui" onclick={() => (aboutOpen = true)}>
    v{ui.version || '?'}
  </Button>
</footer>

<AboutDialog bind:open={aboutOpen} />
