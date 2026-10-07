<script lang="ts">
  import type { Snippet } from 'svelte';
  import { Input } from '$lib/components/ui/input/index.js';
  import { Textarea } from '$lib/components/ui/textarea/index.js';
  import type { DraftField, DraftFields } from '$lib/track-draft';

  interface Props {
    /** The values shown; null = nothing selected (all fields disabled and empty). */
    draft: DraftFields | null;
    editing: boolean;
    errors: Partial<Record<DraftField, string>>;
    oninput: (field: DraftField, value: string) => void;
    /** Element ids are `<idPrefix>-<field>` (and `-error` for the message under it). */
    idPrefix: string;
    /** Extra rows (label + control pairs) placed in the same grid after the fields. */
    children?: Snippet;
  }
  let { draft, editing, errors, oninput, idPrefix, children }: Props = $props();

  const noSelection = $derived(draft === null);

  const textFields: { key: DraftField; label: string }[] = [
    { key: 'band', label: 'Band' },
    { key: 'album', label: 'Album' },
    { key: 'title', label: 'Title' },
  ];
  const lateFields: { key: DraftField; label: string }[] = [
    { key: 'source_url', label: 'Source link' },
    { key: 'copyright', label: 'Copyright' },
  ];

  // Read-only (view mode) fields drop their box; edit mode shows the full input box.
  const viewLook = $derived(!editing && !noSelection ? 'border-transparent bg-transparent dark:bg-transparent' : '');
</script>

{#snippet field(f: { key: DraftField; label: string })}
  <label for="{idPrefix}-{f.key}" class="pt-1.5 text-sm font-medium">{f.label}</label>
  <div class="flex flex-col gap-1">
    <Input
      id="{idPrefix}-{f.key}"
      value={draft ? draft[f.key] : ''}
      disabled={noSelection}
      readonly={!editing}
      class={viewLook}
      aria-invalid={errors[f.key] ? true : undefined}
      aria-describedby={errors[f.key] ? `${idPrefix}-${f.key}-error` : undefined}
      oninput={(e) => oninput(f.key, e.currentTarget.value)}
    />
    {#if errors[f.key]}
      <p id="{idPrefix}-{f.key}-error" class="text-sm text-destructive">{errors[f.key]}</p>
    {/if}
  </div>
{/snippet}

<div class="grid grid-cols-[7rem_minmax(0,1fr)] items-start gap-x-3 gap-y-2">
  {#each textFields as f (f.key)}{@render field(f)}{/each}

  <label for="{idPrefix}-composers" class="pt-1.5 text-sm font-medium">Composers</label>
  <div class="flex flex-col gap-1">
    <Textarea
      id="{idPrefix}-composers"
      rows={3}
      placeholder={editing ? 'One composer per line' : ''}
      value={draft?.composers ?? ''}
      disabled={noSelection}
      readonly={!editing}
      class={viewLook}
      aria-invalid={errors.composers ? true : undefined}
      aria-describedby={errors.composers ? `${idPrefix}-composers-error` : undefined}
      oninput={(e) => oninput('composers', e.currentTarget.value)}
    />
    {#if errors.composers}
      <p id="{idPrefix}-composers-error" class="text-sm text-destructive">{errors.composers}</p>
    {/if}
  </div>

  <label for="{idPrefix}-year" class="pt-1.5 text-sm font-medium">Year</label>
  <div class="flex flex-col gap-1">
    <Input
      id="{idPrefix}-year"
      inputmode="numeric"
      class={`w-28 ${viewLook}`}
      value={draft?.year ?? ''}
      disabled={noSelection}
      readonly={!editing}
      aria-invalid={errors.year ? true : undefined}
      aria-describedby={errors.year ? `${idPrefix}-year-error` : undefined}
      oninput={(e) => oninput('year', e.currentTarget.value)}
    />
    {#if errors.year}
      <p id="{idPrefix}-year-error" class="text-sm text-destructive">{errors.year}</p>
    {/if}
  </div>

  {#each lateFields as f (f.key)}{@render field(f)}{/each}

  {@render children?.()}
</div>
