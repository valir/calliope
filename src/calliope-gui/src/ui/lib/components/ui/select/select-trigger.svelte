<script lang="ts">
	import { Select as SelectPrimitive } from "bits-ui";
	import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		children,
		...restProps
	}: SelectPrimitive.TriggerProps = $props();
</script>

<SelectPrimitive.Trigger
	bind:ref
	data-slot="select-trigger"
	class={cn(
		"border-input dark:bg-input/30 focus-visible:border-ring focus-visible:ring-ring/50 flex h-[3.25rem] w-full items-center justify-between gap-2 rounded-lg border bg-transparent px-3 text-base whitespace-nowrap outline-none transition-colors focus-visible:ring-3 disabled:cursor-not-allowed disabled:opacity-50 data-placeholder:text-muted-foreground [&_svg]:shrink-0",
		className
	)}
	{...restProps}
>
	{@render children?.()}
	<ChevronDownIcon class="size-5 opacity-60" />
</SelectPrimitive.Trigger>
