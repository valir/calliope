<script lang="ts">
	import { Checkbox as CheckboxPrimitive } from "bits-ui";
	import { cn, type WithoutChildrenOrChild } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		checked = $bindable(false),
		class: className,
		...restProps
	}: WithoutChildrenOrChild<CheckboxPrimitive.RootProps> = $props();
</script>

<CheckboxPrimitive.Root
	bind:ref
	bind:checked
	data-slot="checkbox"
	class={cn(
		"border-muted-foreground/60 bg-background data-checked:border-amber-500 data-checked:bg-amber-500 data-checked:text-black focus-visible:border-ring focus-visible:ring-ring/50 peer inline-flex size-7 shrink-0 items-center justify-center rounded-md border-2 outline-none transition-all focus-visible:ring-4 disabled:cursor-not-allowed disabled:opacity-50",
		className
	)}
	{...restProps}
>
	{#snippet children({ checked })}
		{#if checked}
			<svg
				viewBox="0 0 16 16"
				class="size-5"
				fill="none"
				stroke="currentColor"
				stroke-width="2.5"
				stroke-linecap="round"
				stroke-linejoin="round"
				aria-hidden="true"
			>
				<path d="M3 8.5l3.2 3.2L13 4.5" />
			</svg>
		{/if}
	{/snippet}
</CheckboxPrimitive.Root>
