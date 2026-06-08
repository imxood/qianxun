<script lang="ts">
	import { sessionStore } from '$lib/stores/session.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { formatRelativeTime } from '$lib/utils/format';
	import Icon from '../shared/Icon.svelte';

	const active = $derived(sessionStore.active);
	const activePlan = $derived(active ? planStore.bySession(active.id).find((p) => p.status === 'Running') : null);
	const view = $derived(uiStore.activeView);
	const activeSub = $derived(subSessionStore.active);
	const subFollowup = $derived(activeSub ? !subSessionStore.isActive(activeSub) : false);
</script>

<header
	class="h-12 px-4 border-b border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 flex items-center gap-3 flex-shrink-0"
>
	<div class="flex items-center gap-2 min-w-0 flex-1">
		{#if view.kind === 'session' && active}
			<h4 class="text-sm font-medium text-zinc-900 dark:text-zinc-100 truncate">{active.title}</h4>
			<span class="text-xs text-zinc-500">·</span>
			<span class="text-xs text-zinc-500">{active.model}</span>
			{#if active.status === 'Idle' && !activePlan}
				<span class="text-xs text-zinc-500">·</span>
				<span class="text-xs text-zinc-500">已完成 {formatRelativeTime(active.last_active_at)}</span>
			{/if}
			{#if activePlan}
				<span class="text-xs text-zinc-500">·</span>
				<span class="text-xs text-amber-600 dark:text-amber-400">Plan 运行中</span>
			{/if}
		{:else if view.kind === 'sub_session' && activeSub}
			<h4 class="text-sm font-medium text-zinc-900 dark:text-zinc-100 truncate">子会话 · {activeSub.role}</h4>
			<span class="text-xs text-zinc-500">·</span>
			<span class="text-xs text-zinc-500">{activeSub.status.toLowerCase()}</span>
			{#if subFollowup}
				<span class="text-[10px] px-1.5 py-0.5 rounded bg-zinc-200 dark:bg-zinc-800 text-zinc-500 dark:text-zinc-400">追问模式</span>
			{/if}
		{:else if view.kind === 'new'}
			<h4 class="text-sm font-medium text-zinc-500">新会话 · 还没开始</h4>
		{:else}
			<h4 class="text-sm font-medium text-zinc-500">千寻</h4>
		{/if}
	</div>

	<div class="flex items-center gap-1">
		{#if view.kind === 'sub_session' && activeSub}
			<button
				class="h-8 px-2.5 flex items-center gap-1.5 rounded text-xs text-zinc-600 dark:text-zinc-400 hover:bg-zinc-200 dark:hover:bg-zinc-800 hover:text-zinc-900 dark:hover:text-zinc-100"
				onclick={() => sessionStore.switchTo(activeSub.parent_session_id)}
				aria-label="返回主会话"
				title="返回主会话"
			>
				<Icon name="arrow-left" class="w-3.5 h-3.5" />
				<span>返回主会话</span>
			</button>
		{/if}
		{#if uiStore.col3Collapsed}
			<button
				class="p-1.5 rounded hover:bg-zinc-200 dark:hover:bg-zinc-800 text-zinc-500 dark:text-zinc-400"
				aria-label="展开 Inspector"
				title="展开 Inspector"
				onclick={() => uiStore.expandCol3()}
			>
				<Icon name="panel-right-open" class="w-4 h-4" />
			</button>
		{/if}
		<button class="p-1.5 rounded hover:bg-zinc-200 dark:hover:bg-zinc-800 text-zinc-500 dark:text-zinc-400" aria-label="重置 mock" onclick={() => { if (confirm('重置 mock 数据?')) location.reload(); }}>
			<Icon name="refresh-cw" class="w-4 h-4" />
		</button>
	</div>
</header>
