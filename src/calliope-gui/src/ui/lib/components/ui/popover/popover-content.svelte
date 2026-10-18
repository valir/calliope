<script lang="ts">
	import { Popover as PopoverPrimitive } from "bits-ui";
	import PopoverPortal from "./popover-portal.svelte";
	import { cn } from "$lib/utils.js";
	import type { ComponentProps } from "svelte";
	import type { WithoutChildrenOrChild } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		sideOffset = 4,
		align = "center",
		portalProps,
		...restProps
	}: PopoverPrimitive.ContentProps & {
		portalProps?: WithoutChildrenOrChild<ComponentProps<typeof PopoverPortal>>;
	} = $props();
</script>

<PopoverPortal {...portalProps}>
	<PopoverPrimitive.Content
		bind:ref
		data-slot="popover-content"
		{sideOffset}
		{align}
		class={cn(
			"bg-popover text-popover-foreground data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 ring-foreground/10 z-50 w-72 origin-(--bits-popover-content-transform-origin) rounded-lg p-2.5 text-sm shadow-md ring-1 outline-hidden duration-100",
			className
		)}
		{...restProps}
	/>
</PopoverPortal>
