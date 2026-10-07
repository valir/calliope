<script lang="ts">
  import ChevronsDownUpIcon from '@lucide/svelte/icons/chevrons-down-up';
  import ChevronsUpDownIcon from '@lucide/svelte/icons/chevrons-up-down';
  import XIcon from '@lucide/svelte/icons/x';
  import { Button } from '$lib/components/ui/button/index.js';
  import { Input } from '$lib/components/ui/input/index.js';
  import { lib, collapseAll, expandAll, setSearch } from '$lib/library-state.svelte';

  let {
    searchRef = $bindable(null),
    onsearchdown,
  }: { searchRef?: HTMLInputElement | null; onsearchdown?: () => void } = $props();

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      onsearchdown?.();
    }
  }
</script>

<div class="flex flex-col gap-2" data-testid="tree-toolbar">
  <div class="flex gap-2">
    <Button variant="outline" size="sm" disabled={lib.mode === 'edit'} onclick={collapseAll}>
      <ChevronsDownUpIcon aria-hidden="true" /> Collapse all
    </Button>
    <Button variant="outline" size="sm" disabled={lib.mode === 'edit'} onclick={expandAll}>
      <ChevronsUpDownIcon aria-hidden="true" /> Expand all
    </Button>
  </div>
  <div class="flex gap-2">
    <Input
      bind:ref={searchRef}
      type="search"
      aria-label="Search tracks"
      placeholder="Search band, album, title"
      autocomplete="off"
      spellcheck={false}
      disabled={lib.mode === 'edit'}
      value={lib.search}
      oninput={(e) => setSearch(e.currentTarget.value)}
      {onkeydown}
    />
    <Button
      variant="outline"
      size="icon"
      aria-label="Clear search"
      disabled={lib.search === '' || lib.mode === 'edit'}
      onclick={() => {
        setSearch('');
        searchRef?.focus();
      }}
    >
      <XIcon aria-hidden="true" />
    </Button>
  </div>
</div>
