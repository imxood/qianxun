<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import type { Session } from '$lib/types/entity';

	const all = $derived(sessionStore.all);
	const activeId = $derived(uiStore.activeView.kind === 'session' ? uiStore.activeView.session_id : null);

	// 按 created_at 倒序, 取前 6 — 固定位置, 点击不重排
	const recent = $derived(
		[...all].sort((a, b) => b.created_at.localeCompare(a.created_at)).slice(0, 6),
	);

	function statusOf(s: Session): 'running' | 'idle' {
		const runningPlan = planStore.bySession(s.id).find((p) => p.status === 'Running');
		return runningPlan ? 'running' : 'idle';
	}
</script>

<div>
	<div class="px-3 py-1 text-[11px] text-zinc-500 font-medium tracking-wide flex items-center justify-between">
		<span>任务历史</span>
		<span class="text-zinc-400">{all.length}</span>
	</div>
	<div class="px-1.5 space-y-0.5">
		{#each recent as s (s.id)}
			{@const status = statusOf(s)}
			<button
				class={`w-full flex items-center gap-2 px-2 py-1.5 text-[13px] rounded ${activeId === s.id ? 'bg-zinc-200 dark:bg-zinc-800 text-zinc-900 dark:text-zinc-100' : 'text-zinc-600 dark:text-zinc-400 hover:bg-zinc-200 dark:hover:bg-zinc-800'}`}
				onclick={() => sessionStore.switchTo(s.id)}
			>
				<Icon
					name={status === 'running' ? 'folder-open' : 'folder'}
					class="w-3.5 h-3.5 flex-shrink-0 {status === 'running' ? (activeId === s.id ? 'text-amber-500' : 'text-amber-600 dark:text-amber-400') : 'text-zinc-500'}"
				/>
				<span class="flex-1 text-left truncate">{s.title}</span>
				{#if status === 'running'}
					<StatusDot color="sky" pulsing={true} />
				{/if}
			</button>
		{/each}
		<button
			class="w-full flex items-center gap-2 px-2 py-1.5 text-[13px] text-zinc-500 dark:text-zinc-500 hover:bg-zinc-200/50 dark:hover:bg-zinc-800/50 rounded"
			onclick={() => uiStore.pushToast({ kind: 'info', title: '完整历史记录功能开发中', timeout_ms: 2000 })}
		>
			<Icon name="more-horizontal" class="w-3.5 h-3.5 flex-shrink-0" />
			<span class="flex-1 text-left">更多 ({all.length - 6})</span>
		</button>
	</div>
</div>
