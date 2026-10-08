<script lang="ts">
	import { Slider as SliderPrimitive } from "bits-ui";
	import { cn } from "$lib/utils.js";

	type Props = Omit<
		Extract<SliderPrimitive.RootProps, { type: "single" }>,
		"type" | "value" | "children" | "child" | "aria-label" | "aria-valuetext"
	> & {
		value?: number;
		"aria-label": string;
		"aria-valuetext"?: string;
		/** Draws a mark at this value (e.g. 0 dB). */
		tick?: number;
	};

	let {
		ref = $bindable(null),
		value = $bindable(0),
		class: className,
		min = 0,
		max = 100,
		step = 1,
		tick,
		"aria-label": ariaLabel,
		"aria-valuetext": ariaValuetext,
		...restProps
	}: Props = $props();

	const tickPct = $derived(
		tick === undefined || max === min
			? null
			: Math.min(100, Math.max(0, ((tick - min) / (max - min)) * 100))
	);
</script>

<SliderPrimitive.Root
	bind:ref
	bind:value
	type="single"
	{min}
	{max}
	{step}
	data-slot="slider"
	class={cn(
		"relative flex h-8 w-full touch-none items-center select-none data-disabled:opacity-50",
		className
	)}
	{...restProps}
>
	<span
		data-slot="slider-track"
		class="bg-muted relative h-2 w-full grow overflow-hidden rounded-full"
	>
		<SliderPrimitive.Range data-slot="slider-range" class="bg-amber-500 absolute h-full" />
	</span>
	{#if tickPct !== null}
		<span
			data-slot="slider-tick"
			data-tick={tick}
			class="bg-foreground pointer-events-none absolute top-1 h-6 w-0.5 -translate-x-1/2 rounded-full opacity-70"
			style:left="{tickPct}%"
		></span>
	{/if}
	<SliderPrimitive.Thumb
		index={0}
		aria-label={ariaLabel}
		aria-valuetext={ariaValuetext}
		data-slot="slider-thumb"
		class="border-amber-500 bg-background focus-visible:ring-ring/50 focus-visible:border-ring block size-6 shrink-0 rounded-full border-2 shadow-sm outline-none transition-[box-shadow] focus-visible:ring-4 disabled:pointer-events-none"
	/>
</SliderPrimitive.Root>
