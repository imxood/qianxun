<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import type { Plan } from '$lib/types/entity';

	let { plan }: { plan: Plan } = $props();

	const tasks = $derived(plan.contract.tasks);
	const subSessions = $derived(subSessionStore.byPlan(plan.id));
	const progress = $derived(planStore.progressOf(plan));
	const isRunning = $derived(plan.status === 'Running');

	function statusOf(taskId: string, idx: number): 'done' | 'running' | 'pending' | 'failed' {
		const sub = subSessions[idx];
		if (!sub) {
			return idx < progress.done ? 'done' : 'pending';
		}
		const s = sub.status;
		if (s === 'Done') return 'done';
		if (s === 'Active') return 'running';
		if (s === 'Failed') return 'failed';
		if (s === 'Aborted') return 'failed';
		return 'pending';
	}

	function roleClass(role: string) {
		// 不同 role 不同色
		const map: Record<string, string> = {
			coder: 'bg-sky-500/15 text-sky-700 dark:text-sky-300',
			tester: 'bg-amber-500/15 text-amber-700 dark:text-amber-300',
			researcher: 'bg-violet-500/15 text-violet-700 dark:text-violet-300',
			verifier: 'bg-emerald-500/15 text-emerald-700 dark:text-emerald-300',
		};
		return map[role] || 'bg-zinc-200 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300';
	}
</script>

<div class="rounded-lg border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900/50 overflow-hidden">
	<!-- 头部 -->
	<div class="px-4 py-3 flex items-center gap-3 border-b border-zinc-200 dark:border-zinc-800">
		<div class="w-8 h-8 rounded-md bg-amber-100 dark:bg-amber-500/10 flex items-center justify-center">
			<Icon name="layers" class="w-4 h-4 text-amber-600 dark:text-amber-400" />
		</div>
		<div class="flex-1 min-w-0">
			<div class="flex items-center gap-2 flex-wrap">
				<h5 class="text-sm font-medium text-zinc-900 dark:text-zinc-100 truncate">{plan.contract.name}</h5>
				<span
					class="text-[10px] px-1.5 py-0.5 rounded font-medium"
					class:bg-sky-500={isRunning}
					class:text-sky-100={isRunning}
					class:bg-emerald-500={plan.status === 'Done'}
					class:text-emerald-100={plan.status === 'Done'}
					class:bg-zinc-500={plan.status === 'Aborted' || plan.status === 'Failed'}
					class:text-zinc-100={plan.status === 'Aborted' || plan.status === 'Failed'}
				>
					{plan.status.toLowerCase()} · {progress.done}/{progress.total}
				</span>
			</div>
			{#if isRunning}
				<p class="text-xs text-zinc-500 mt-0.5">{tasks.length} tasks · 等待 verifier</p>
			{:else if plan.status === 'Done' && plan.result}
				<p class="text-xs text-zinc-500 mt-0.5">{plan.result.deliverables.length} 项交付</p>
			{/if}
		</div>
		{#if isRunning}
			<button
				class="text-[11px] px-2 py-0.5 rounded text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-800 hover:text-zinc-700 dark:hover:text-zinc-300"
				onclick={() => planStore.cancel(plan.id)}
			>
				取消
			</button>
		{/if}
	</div>

	<!-- 任务列表 -->
	<div class="px-4 py-2 space-y-0.5">
		{#each tasks as task, i (task.id)}
			{@const st = statusOf(task.id, i)}
			<div class="flex items-center gap-3 px-2 py-1.5 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800/30">
				{#if st === 'done'}
					<Icon name="check-circle-2" class="w-4 h-4 text-emerald-500 flex-shrink-0" />
				{:else if st === 'running'}
					<Icon name="loader" class="w-4 h-4 text-sky-500 flex-shrink-0 animate-spin" />
				{:else if st === 'failed'}
					<Icon name="x-circle" class="w-4 h-4 text-rose-500 flex-shrink-0" />
				{:else}
					<Icon name="hash" class="w-4 h-4 text-zinc-400 flex-shrink-0" />
				{/if}
				<div class="flex-1 min-w-0">
					<div class="flex items-center gap-2 flex-wrap">
						<span
							class="text-sm truncate"
							class:text-zinc-900={st === 'running'}
							class:dark:text-zinc-100={st === 'running'}
							class:text-zinc-700={st !== 'running'}
							class:dark:text-zinc-300={st !== 'running'}
						>
							{task.title}
						</span>
						<span class="text-[10px] px-1.5 py-0.5 rounded {roleClass(task.assigned_to)}">{task.assigned_to}</span>
						{#if st === 'done' && task.verified_by}
							<span class="text-[10px] text-emerald-600 dark:text-emerald-400 flex items-center gap-0.5">
								<Icon name="check" class="w-2.5 h-2.5" /> PASS
							</span>
						{:else if st === 'running'}
							<span class="text-[10px] text-zinc-500">verifier 等待中</span>
						{/if}
					</div>
					<p class="text-xs text-zinc-500 mt-0.5">
						+ 1 file · {st === 'running' ? '已 4 min' : '12 min'}
					</p>
				</div>
				<button
					class="text-xs text-amber-600 dark:text-amber-400 hover:underline opacity-70 hover:opacity-100 flex-shrink-0"
					onclick={() => {
						const sub = subSessions[i];
						if (sub) subSessionStore.open(sub.id);
					}}
				>打开子会话</button>
			</div>
		{/each}
	</div>

	<!-- 底部 summary -->
	<div class="px-4 py-2.5 border-t border-zinc-200 dark:border-zinc-800 flex items-center gap-2 bg-zinc-50/50 dark:bg-zinc-900/30">
		<span class="text-xs text-zinc-500">已修改 {planStore.getChangedFiles(plan.id).length} 文件</span>
		<span class="text-zinc-300 dark:text-zinc-700">·</span>
		<span class="text-xs text-zinc-500">任务进度 {progress.done}/{progress.total}</span>
		<span class="text-zinc-300 dark:text-zinc-700">·</span>
		<span class="text-xs text-zinc-500">verifier {isRunning ? '待就绪' : '已通过'}</span>
		<div class="flex-1"></div>
		<button class="text-xs px-2.5 py-1 rounded text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800">查看日志</button>
	</div>
</div>
