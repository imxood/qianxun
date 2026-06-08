<script lang="ts">
	import Icon from './Icon.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';

	const toasts = $derived(uiStore.toasts);

	function borderClass(kind: string) {
		if (kind === 'success') return 'border-emerald-500';
		if (kind === 'warn') return 'border-amber-500';
		if (kind === 'error') return 'border-rose-500';
		return 'border-sky-500';
	}

	function iconColorClass(kind: string) {
		if (kind === 'success') return 'text-emerald-500';
		if (kind === 'warn') return 'text-amber-500';
		if (kind === 'error') return 'text-rose-500';
		return 'text-sky-500';
	}
</script>

<div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2 max-w-sm">
	{#each toasts as t (t.id)}
		<div
			class="rounded-lg border px-4 py-3 shadow-lg backdrop-blur bg-white/95 dark:bg-zinc-900/95 flex items-start gap-2.5 {borderClass(t.kind)}"
		>
			<Icon
				name="info"
				class="w-4 h-4 mt-0.5 flex-shrink-0 {iconColorClass(t.kind)}"
			/>
			<div class="flex-1 min-w-0">
				<div class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{t.title}</div>
				{#if t.description}
					<div class="text-xs text-zinc-500 dark:text-zinc-400 mt-0.5">{t.description}</div>
				{/if}
				{#if t.action}
					<button
						class="text-xs text-amber-600 dark:text-amber-400 hover:underline mt-1"
						onclick={() => {
							if (t.action?.on_click) t.action.on_click();
							uiStore.dismissToast(t.id);
						}}
					>
						{t.action.label}
					</button>
				{/if}
			</div>
			<button
				class="text-zinc-400 hover:text-zinc-600 dark:hover:text-zinc-200 flex-shrink-0"
				onclick={() => uiStore.dismissToast(t.id)}
				aria-label="关闭"
			>
				<Icon name="x" class="w-3.5 h-3.5" />
			</button>
		</div>
	{/each}
</div>
