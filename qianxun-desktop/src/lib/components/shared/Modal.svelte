<script lang="ts">
	import Icon from './Icon.svelte';

	let { open, onClose, title, children, maxWidth = 'max-w-md' }: {
		open: boolean;
		onClose: () => void;
		title: string;
		children?: any;
		maxWidth?: string;
	} = $props();
</script>

{#if open}
	<div
		class="fixed inset-0 z-50 bg-zinc-950/70 backdrop-blur-sm flex items-center justify-center"
		role="dialog"
		aria-modal="true"
	>
		<div class="w-full {maxWidth} mx-4 rounded-xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 shadow-2xl">
			<div class="px-5 py-4 border-b border-zinc-200 dark:border-zinc-800 flex items-center justify-between">
				<h3 class="text-sm font-semibold text-zinc-900 dark:text-zinc-100">{title}</h3>
				<button
					class="p-1 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-500"
					onclick={onClose}
					aria-label="关闭"
				>
					<Icon name="x" class="w-4 h-4" />
				</button>
			</div>
			<div class="p-5">
				{@render children?.()}
			</div>
		</div>
	</div>
{/if}
