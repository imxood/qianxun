<script lang="ts">
	// 简化版 Select (Stage 7a 不用 bits-ui).
	// 受控 value + onchange 回调.

	import { cn } from '$lib/utils';

	type Option = { value: string; label: string };

	type Props = {
		value: string;
		onchange?: (value: string) => void;
		options: Option[];
		class?: string;
		placeholder?: string;
		disabled?: boolean;
		id?: string;
	};

	let {
		value = $bindable(''),
		onchange,
		options,
		class: className = '',
		placeholder,
		disabled = false,
		id
	}: Props = $props();
</script>

<select
	{id}
	{disabled}
	class={cn(
		'border-input bg-background ring-offset-background focus-visible:ring-ring flex h-9 w-full rounded-md border px-3 py-1 text-sm shadow-sm focus-visible:ring-1 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50',
		className
	)}
	bind:value
	onchange={(e) => {
		const v = (e.currentTarget as HTMLSelectElement).value;
		onchange?.(v);
	}}
>
	{#if placeholder}
		<option value="" disabled>{placeholder}</option>
	{/if}
	{#each options as opt (opt.value)}
		<option value={opt.value}>{opt.label}</option>
	{/each}
</select>
