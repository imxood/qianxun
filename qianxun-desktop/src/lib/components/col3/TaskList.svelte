<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import type { Plan, PlanStatus } from '$lib/types/entity';

	let { plan }: { plan: Plan } = $props();
	const tasks = $derived(plan.contract.tasks);
	const subs = $derived(subSessionStore.byPlan(plan.id));
	const progress = $derived(planStore.progressOf(plan));

	function statusAt(idx: number): 'done' | 'running' | 'pending' | 'failed' {
		const sub = subs[idx];
		if (!sub) return idx < progress.done ? 'done' : 'pending';
		if (sub.status === 'Done') return 'done';
		if (sub.status === 'Active') return 'running';
		if (sub.status === 'Failed' || sub.status === 'Aborted') return 'failed';
		return 'pending';
	}

	function openSub(idx: number) {
		const sub = subs[idx];
		if (sub) subSessionStore.open(sub.id);
	}
</script>

<div class="p-3 border-b border-zinc-200 dark:border-zinc-800">
	<div class="flex items-center justify-between mb-2">
		<h6 class="text-xs font-semibold text-zinc-500 dark:text-zinc-400 uppercase tracking-wider">Tasks</h6>
		<span class="text-[10px] text-zinc-500">{progress.done}/{progress.total}</span>
	</div>
	<div class="space-y-1">
		{#each tasks as task, i (task.id)}
			{@const st = statusAt(i)}
			<div class="px-2 py-1.5 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800 flex items-center gap-2 group cursor-pointer" onclick={() => openSub(i)} role="button" tabindex="0">
				{#if st === 'done'}
					<Icon name="check-circle-2" class="w-3.5 h-3.5 text-emerald-500 flex-shrink-0" />
				{:else if st === 'running'}
					<Icon name="loader" class="w-3.5 h-3.5 text-sky-500 flex-shrink-0 animate-spin" />
				{:else}
					<Icon name="hash" class="w-3.5 h-3.5 text-zinc-400 flex-shrink-0" />
				{/if}
				<span class="text-xs text-zinc-700 dark:text-zinc-300 truncate flex-1">{task.title}</span>
				{#if st === 'running'}<StatusDot color="sky" pulsing={true} />{/if}
				{#if st === 'done'}
					<span class="text-[10px] text-emerald-500 opacity-0 group-hover:opacity-100">子会话</span>
				{/if}
			</div>
		{/each}
	</div>
</div>
