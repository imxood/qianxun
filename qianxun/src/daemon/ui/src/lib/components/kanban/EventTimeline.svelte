<script lang="ts">
	// Kanban 事件时间线 (滚动列表, 2026-06-04 阶段 3)
	import type { KanbanEvent } from '$lib/types/kanban';
	type Props = { events: KanbanEvent[]; max?: number };
	let { events, max = 50 }: Props = $props();
	const visible = $derived(events.slice(0, max));

	function fmtKind(kind: string): string {
		// snake_case → Title Case
		return kind.split('_').map(w => w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
	}
	function fmtTime(s: string): string {
		try {
			return new Date(s).toLocaleTimeString();
		} catch {
			return s;
		}
	}
</script>

<div class="bg-card/50 flex flex-col gap-1 overflow-y-auto rounded border p-2 text-xs" data-testid="event-timeline">
	{#if visible.length === 0}
		<div class="text-muted-foreground py-2 text-center text-[10px]">暂无事件</div>
	{:else}
		{#each visible as ev (ev.id)}
			<div class="flex items-start gap-2 border-b py-1 last:border-b-0" data-testid="event-{ev.id}">
				<span class="text-muted-foreground w-12 shrink-0 font-mono text-[10px]">{fmtTime(ev.created_at)}</span>
				<span class="shrink-0 font-semibold text-[10px]">{fmtKind(ev.kind)}</span>
				{#if ev.task_id}
					<span class="text-muted-foreground truncate text-[10px]">→ {ev.task_id}</span>
				{/if}
			</div>
		{/each}
	{/if}
</div>
