<script lang="ts">
	import { Select as SelectPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	let {
		ref = $bindable(null),
		class: className,
		sideOffset = 4,
		portalProps,
		children,
		...restProps
	}: SelectPrimitive.ContentProps & { portalProps?: SelectPrimitive.PortalProps } = $props();
</script>

<SelectPrimitive.Portal {...portalProps}>
	<SelectPrimitive.Content
		bind:ref
		{sideOffset}
		data-slot="select-content"
		class={cn(
			"bg-popover text-popover-foreground z-50 max-h-80 min-w-(--bits-select-anchor-width) overflow-y-auto rounded-lg border p-1 shadow-md",
			className
		)}
		{...restProps}
	>
		<SelectPrimitive.Viewport>
			{@render children?.()}
		</SelectPrimitive.Viewport>
	</SelectPrimitive.Content>
</SelectPrimitive.Portal>
