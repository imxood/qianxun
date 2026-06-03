<script lang="ts">
	// Kanban task 卡片 (2026-06-04 阶段 3)
	import type { Task } from '$lib/types/kanban';
	import StatusBadge from './StatusBadge.svelte';
	type Props = { task: Task; onclick?: () => void };
	let { task, onclick }: Props = $props();
</script>

<button
	type="button"
	class="hover:bg-accent/30 w-full rounded border bg-card p-2 text-left text-xs transition-colors"
	{onclick}
	data-testid="task-card-{task.id}"
>
	<div class="mb-1 line-clamp-2 font-medium">{task.title}</div>
	<div class="text-muted-foreground flex items-center justify-between text-[10px]">
		<span class="truncate">{task.assignee_role}</span>
		<StatusBadge status={task.status} size="sm" />
	</div>
	{#if task.priority !== 128}
		<div class="text-muted-foreground mt-1 text-[10px]">
			priority: {task.priority}
		</div>
	{/if}
</button>
