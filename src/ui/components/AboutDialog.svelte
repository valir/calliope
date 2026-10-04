<script lang="ts">
  import * as Dialog from '$lib/components/ui/dialog/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import { ui } from '$lib/app-state.svelte';

  let { open = $bindable(false) }: { open: boolean } = $props();
</script>

<Dialog.Root bind:open>
  <Dialog.Content showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>calliope-gui</Dialog.Title>
      <Dialog.Description>Version {ui.version || '?'}</Dialog.Description>
    </Dialog.Header>
    <p>(c) 2026 Valentin Rusu</p>
    <p class="text-muted-foreground">Calliope helps musicians improve their music playing skills.</p>
    <Dialog.Footer>
      <Dialog.Close>
        {#snippet child({ props })}
          <Button {...props}>Close</Button>
        {/snippet}
      </Dialog.Close>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
