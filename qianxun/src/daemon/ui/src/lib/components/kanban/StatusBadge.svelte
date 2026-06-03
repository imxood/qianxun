<script lang="ts">
	// Kanban 状态徽章 (5 状态 + 24 事件 kind, 2026-06-04 阶段 3)
	import type { TaskStatus } from '$lib/types/kanban';
	type Props = { status: TaskStatus; size?: 'sm' | 'md' };
	let { status, size = 'md' }: Props = $props();

	const COLOR_MAP: Record<TaskStatus, { bg: string; text: string; label: string }> = {
		triage:      { bg: 'bg-slate-100 dark:bg-slate-800',   text: 'text-slate-700 dark:text-slate-300',  label: 'Triage' },
		ready:       { bg: 'bg-blue-100 dark:bg-blue-950',    text: 'text-blue-700 dark:text-blue-300',     label: 'Ready' },
		in_progress: { bg: 'bg-amber-100 dark:bg-amber-950',  text: 'text-amber-700 dark:text-amber-300',   label: 'In Progress' },
		done:        { bg: 'bg-emerald-100 dark:bg-emerald-950', text: 'text-emerald-700 dark:text-emerald-300', label: 'Done' },
		blocked:     { bg: 'bg-red-100 dark:bg-red-950',      text: 'text-red-700 dark:text-red-300',       label: 'Blocked' },
		cancelled:   { bg: 'bg-zinc-200 dark:bg-zinc-700',    text: 'text-zinc-600 dark:text-zinc-300',     label: 'Cancelled' },
		failed:      { bg: 'bg-rose-200 dark:bg-rose-900',    text: 'text-rose-700 dark:text-rose-300',     label: 'Failed' }
	};
	const cfg = $derived(COLOR_MAP[status] ?? COLOR_MAP.triage);
</script>

<span
	class="inline-flex items-center gap-1 rounded-full font-semibold {cfg.bg} {cfg.text} {size === 'sm' ? 'px-1.5 py-0.5 text-[10px]' : 'px-2 py-0.5 text-xs'}"
	data-testid="status-{status}"
>
	{cfg.label}
</span>
