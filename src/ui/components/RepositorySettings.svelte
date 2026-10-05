<script lang="ts">
  import { onMount } from 'svelte';
  import * as Card from '$lib/components/ui/card/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import {
    chooseRepositoryRoot,
    frontendLog,
    getRepository,
    resetRepositoryRoot,
    setRepositoryRoot,
    type PickedRoot,
    type RepoInfo,
  } from '$lib/ipc';
  import { lib, resetLibrary } from '$lib/library-state.svelte';

  let info = $state<RepoInfo | null>(null);
  let error = $state('');
  let busy = $state(false);
  let confirmOpen = $state(false);
  let pending = $state<PickedRoot | null>(null);

  const editing = () => lib.mode === 'edit';
  const editBlock = 'Finish or cancel the track edit in the Library before changing the track repository.';

  async function refresh(): Promise<void> {
    try {
      info = await getRepository();
    } catch (err) {
      error = `Could not read the track repository setting: ${String(err)}`;
      void frontendLog(`error get_repository: ${String(err)}`);
    }
  }

  onMount(() => void refresh());

  function statusText(i: RepoInfo): string {
    if (i.status === 'missing') return 'This folder does not exist. Choose another one or use the default.';
    if (i.status === 'newer') return 'This repository was written by a newer Calliope and cannot be used.';
    return '';
  }

  function applied(next: RepoInfo): void {
    info = next;
    resetLibrary();
    void frontendLog(`repository root=${next.root} default=${next.is_default}`);
  }

  async function choose(): Promise<void> {
    if (busy) return;
    error = '';
    if (editing()) {
      error = editBlock;
      return;
    }
    busy = true;
    try {
      const picked = await chooseRepositoryRoot();
      if (!picked) return;
      if (picked.status === 'newer') {
        error = `"${picked.root}" was written by a newer Calliope and cannot be used as the track repository.`;
        return;
      }
      if (picked.status === 'other') {
        pending = picked;
        confirmOpen = true;
        return;
      }
      await apply(picked);
    } catch (err) {
      error = `Could not choose the folder: ${String(err)}`;
      void frontendLog(`error choose_repository_root: ${String(err)}`);
    } finally {
      busy = false;
    }
  }

  async function apply(picked: PickedRoot): Promise<void> {
    try {
      applied(await setRepositoryRoot(picked.token));
    } catch (err) {
      error = `Could not use the folder: ${String(err)}`;
      void frontendLog(`error set_repository_root: ${String(err)}`);
    }
  }

  async function onAnswer(yes: boolean): Promise<void> {
    const picked = pending;
    pending = null;
    if (!yes || !picked) return;
    if (editing()) {
      error = editBlock;
      return;
    }
    busy = true;
    try {
      await apply(picked);
    } finally {
      busy = false;
    }
  }

  async function useDefault(): Promise<void> {
    if (busy) return;
    error = '';
    if (editing()) {
      error = editBlock;
      return;
    }
    busy = true;
    try {
      applied(await resetRepositoryRoot());
    } catch (err) {
      error = `Could not reset the folder: ${String(err)}`;
      void frontendLog(`error reset_repository_root: ${String(err)}`);
    } finally {
      busy = false;
    }
  }
</script>

<Card.Root>
  <Card.Header>
    <Card.Title>Track repository</Card.Title>
    <Card.Description>The folder where Calliope keeps your tracks. Changing it never moves or copies tracks.</Card.Description>
  </Card.Header>
  <Card.Content class="flex flex-col gap-3">
    {#if info}
      <p>
        <span class="select-text break-all font-mono" data-testid="repository-root">{info.root}</span>
        {#if info.is_default}<span class="text-muted-foreground"> (default)</span>{/if}
      </p>
      {#if statusText(info)}
        <p class="text-muted-foreground" data-testid="repository-status">{statusText(info)}</p>
      {/if}
    {/if}
    {#if error}
      <p role="alert" class="text-destructive">{error}</p>
    {/if}
    <div class="flex gap-3">
      <Button variant="outline" disabled={busy} onclick={choose}>Choose folder…</Button>
      <Button variant="outline" disabled={busy} onclick={useDefault}>Use default</Button>
    </div>
  </Card.Content>
</Card.Root>

<ConfirmDialog
  bind:open={confirmOpen}
  message={pending
    ? `Use "${pending.root}" as the track repository? Calliope will add a "tracks" folder and a "calliope-repository.json" file there. Existing files are not changed.`
    : ''}
  onanswer={onAnswer}
/>
