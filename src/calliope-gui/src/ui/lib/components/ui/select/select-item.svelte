<script lang="ts">
	import { Select as SelectPrimitive } from "bits-ui";
	import CheckIcon from "@lucide/svelte/icons/check";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		value,
		label,
		children: childrenProp,
		...restProps
	}: SelectPrimitive.ItemProps = $props();
</script>

<SelectPrimitive.Item
	bind:ref
	{value}
	{label}
	data-slot="select-item"
	class={cn(
		"data-highlighted:bg-accent data-highlighted:text-accent-foreground relative flex min-h-12 w-full cursor-default items-center gap-2 rounded-md py-2 pr-9 pl-3 text-base outline-none select-none data-disabled:pointer-events-none data-disabled:opacity-50",
		className
	)}
	{...restProps}
>
	{#snippet children({ selected, highlighted })}
		{#if childrenProp}
			{@render childrenProp({ selected, highlighted })}
		{:else}
			{label ?? value}
		{/if}
		{#if selected}
			<span class="absolute right-3 flex size-4 items-center justify-center"><CheckIcon class="size-4" /></span>
		{/if}
	{/snippet}
</SelectPrimitive.Item>
