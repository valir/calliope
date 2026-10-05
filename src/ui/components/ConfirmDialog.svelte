<script lang="ts">
  import * as AlertDialog from '$lib/components/ui/alert-dialog/index.js';
  import { Button } from '$lib/components/ui/button/index.js';

  let {
    open = $bindable(false),
    message,
    onanswer,
  }: { open: boolean; message: string; onanswer: (yes: boolean) => void } = $props();

  let noButton = $state<HTMLElement | null>(null);

  let answered = false;

  $effect(() => {
    if (open) answered = false;
  });

  function answer(yes: boolean) {
    if (answered) return;
    answered = true;
    open = false;
    onanswer(yes);
  }

  // Closing by any other route (Escape, outside click) counts as "No".
  function onOpenChange(next: boolean) {
    if (!next) answer(false);
  }
</script>

<AlertDialog.Root bind:open {onOpenChange}>
  <AlertDialog.Content
    onOpenAutoFocus={(e) => {
      e.preventDefault();
      noButton?.focus();
    }}
  >
    <AlertDialog.Header>
      <AlertDialog.Title>Please confirm</AlertDialog.Title>
      <AlertDialog.Description>{message}</AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel bind:ref={noButton} onclick={() => answer(false)}>No</AlertDialog.Cancel>
      <AlertDialog.Action onclick={() => answer(true)}>Yes</AlertDialog.Action>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
