<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { formatRelativeTime } from '$lib/utils/format';

	function statusHint(status: string): { label: string; tone: string; tip: string } {
		if (status === 'Active')
			return { label: '执行中', tone: 'text-sky-600 dark:text-sky-400', tip: '可发消息, 会进入任务流' };
		if (status === 'Done')
			return { label: '已完成', tone: 'text-emerald-600 dark:text-emerald-400', tip: '可追问, 不会重新执行' };
		if (status === 'Failed')
			return { label: '失败', tone: 'text-rose-600 dark:text-rose-400', tip: '可追问或重试' };
		if (status === 'Aborted')
			return { label: '中止', tone: 'text-zinc-500', tip: '可追问, 历史保留' };
		return { label: '只读', tone: 'text-zinc-500', tip: '只读档案' };
	}
</script>

{#if uiStore.activeView.kind === 'sub_session'}
	{@const sub = subSessionStore.active}
	{#if sub}
		{@const hint = statusHint(sub.status)}
		<div class="p-3 border-b border-zinc-200 dark:border-zinc-800 space-y-3">
			<div>
				<div class="flex items-center gap-2 mb-2">
					<Icon name="bot" class="w-4 h-4 text-sky-500" />
					<h6 class="text-sm font-medium text-zinc-900 dark:text-zinc-100">{sub.role} · {sub.id.slice(-8)}</h6>
				</div>
				<div class="space-y-1.5 text-xs">
					<div class="flex items-center justify-between">
						<span class="text-zinc-500">状态</span>
						<span class={hint.tone}>{hint.label}</span>
					</div>
					<div class="flex items-center justify-between">
						<span class="text-zinc-500">已运行</span>
						<span class="text-zinc-700 dark:text-zinc-300">{sub.started_at ? formatRelativeTime(sub.started_at) : '-'}</span>
					</div>
					<div class="flex items-center justify-between">
						<span class="text-zinc-500">上下文</span>
						<span class="text-zinc-700 dark:text-zinc-300">独立 ({sub.messages.length} 消息)</span>
					</div>
					<p class="text-[11px] text-zinc-400 dark:text-zinc-500 italic mt-1">{hint.tip}</p>
				</div>
			</div>
			<div>
				<h6 class="text-xs font-semibold text-zinc-500 dark:text-zinc-400 uppercase tracking-wider mb-1.5">父 Plan</h6>
				<div class="px-3 py-2 rounded-md border border-zinc-200 dark:border-zinc-800 bg-zinc-50 dark:bg-zinc-950 text-xs">
					<div class="text-zinc-700 dark:text-zinc-300">实现 JWT 用户认证</div>
					<div class="text-zinc-500 mt-0.5">3 tasks · 2/3 done</div>
				</div>
			</div>
		</div>
	{/if}
{/if}
