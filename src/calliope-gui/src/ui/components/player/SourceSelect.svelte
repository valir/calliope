<script lang="ts">
  import * as Select from '$lib/components/ui/select/index.js';

  let {
    sources,
    value,
    disabled = false,
    onchange,
  }: {
    sources: { key: string; label: string }[];
    value: string;
    disabled?: boolean;
    onchange: (key: string) => void;
  } = $props();

  const current = $derived(sources.find((s) => s.key === value)?.label ?? '');
</script>

<Select.Root
  type="single"
  {value}
  {disabled}
  items={sources.map((s) => ({ value: s.key, label: s.label }))}
  onValueChange={(v) => {
    if (v && v !== value) onchange(v);
  }}
>
  <Select.Trigger aria-label="Audio source" class="w-60" data-testid="source-select">
    <span class="truncate">{current || 'No source'}</span>
  </Select.Trigger>
  <Select.Content>
    {#each sources as s (s.key)}
      <Select.Item value={s.key} label={s.label} />
    {/each}
  </Select.Content>
</Select.Root>
