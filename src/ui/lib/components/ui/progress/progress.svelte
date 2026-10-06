<script lang="ts">
	import { Progress as ProgressPrimitive } from "bits-ui";
	import { cn, type WithoutChildrenOrChild } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		max = 100,
		value,
		...restProps
	}: WithoutChildrenOrChild<ProgressPrimitive.RootProps> = $props();

	// null / undefined = indeterminate (a pulsing full-width bar).
	const percent = $derived(value == null ? null : Math.min(100, Math.max(0, (value / max) * 100)));
</script>

<ProgressPrimitive.Root
	bind:ref
	data-slot="progress"
	class={cn("bg-muted relative h-2 w-full overflow-hidden rounded-full", className)}
	{value}
	{max}
	{...restProps}
>
	<!-- style: directive (CSSOM), not a style attribute: the production CSP has no inline styles. -->
	<div
		data-slot="progress-indicator"
		class={cn("bg-primary h-full transition-all", percent === null && "animate-pulse")}
		style:width={percent === null ? "100%" : `${percent}%`}
	></div>
</ProgressPrimitive.Root>
