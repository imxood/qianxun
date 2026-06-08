<script lang="ts">
	import { planStore } from '$lib/stores/plan.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import { formatRelativeTime } from '$lib/utils/format';
	import Icon from '../shared/Icon.svelte';

	const activePlan = $derived(planStore.active);
</script>

<div class="p-3 border-b border-zinc-200 dark:border-zinc-800">
	{#if activePlan}
		<div class="flex items-center gap-2 mb-2">
			<Icon name="layers" class="w-4 h-4 text-amber-500" />
			<h6 class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{activePlan.contract.name}</h6>
		</div>
		<div class="space-y-1.5 text-xs">
			<div class="flex items-center justify-between">
				<span class="text-zinc-500">启动于</span>
				<span class="text-zinc-700 dark:text-zinc-300">{activePlan.started_at ? formatRelativeTime(activePlan.started_at) : '-'}</span>
			</div>
			<div class="flex items-center justify-between">
				<span class="text-zinc-500">超时</span>
				<span class="text-zinc-700 dark:text-zinc-300">{Math.round(activePlan.contract.timeout_ms / 60000)} min</span>
			</div>
			<div class="flex items-center justify-between">
				<span class="text-zinc-500">verifier</span>
				<span class="text-zinc-700 dark:text-zinc-300">tester + code-reviewer</span>
			</div>
			<div class="flex items-center justify-between">
				<span class="text-zinc-500">依赖</span>
				<span class="text-zinc-700 dark:text-zinc-300 font-mono text-[10px]">
					t0 → t1 → t2
				</span>
			</div>
		</div>
	{:else}
		<p class="text-xs text-zinc-500">当前没有 active plan</p>
	{/if}
</div>
