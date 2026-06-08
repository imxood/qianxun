<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import type { Plan, ChangeKind } from '$lib/types/entity';

	let { plan }: { plan: Plan } = $props();
	const files = $derived(planStore.getChangedFiles(plan.id));

	function kindColor(k: ChangeKind): string {
		if (k === '+') return 'text-emerald-600 dark:text-emerald-400';
		if (k === '~') return 'text-amber-600 dark:text-amber-400';
		return 'text-rose-600 dark:text-rose-400';
	}
</script>

<div class="p-3">
	<h6 class="text-xs font-semibold text-zinc-500 dark:text-zinc-400 uppercase tracking-wider mb-2">Changed files</h6>
	<div class="space-y-0.5 text-xs font-mono">
		{#each files as f (f.path)}
			<div class="flex items-center gap-2 px-1.5 py-1 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800/30">
				<span class="font-bold w-3 {kindColor(f.kind)}">{f.kind}</span>
				<span class="text-zinc-700 dark:text-zinc-300 truncate">{f.path}</span>
			</div>
		{/each}
		{#if files.length === 0}
			<p class="text-xs text-zinc-400 dark:text-zinc-600 px-1.5">暂无变更</p>
		{/if}
	</div>
</div>
