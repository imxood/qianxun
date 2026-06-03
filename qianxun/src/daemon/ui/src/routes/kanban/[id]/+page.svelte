<script lang="ts">
	// /kanban/{id} — 5 列 board 详情视图 (triage/ready/in_progress/done/blocked)
	// 实时 SSE 刷新 (KanbanTaskAssigned/Progress/Completed/Spawned/BlackboardUpdate)
	// 2026-06-04 阶段 3, MVP-3 落地
	import { onMount, onDestroy } from 'svelte';
	import { page } from '$app/state';
	import { ArrowLeft, Plus, X } from '@lucide/svelte';
	import { getBoard, listBoardTasks, listBoardEvents, createTask, cancelTask } from '$lib/api/kanban';
	import { connectSse, type SseClient } from '$lib/sse/client';
	import { parseKanbanEvent } from '$lib/sse/kanban_parser';
	import StatusBadge from '$lib/components/kanban/StatusBadge.svelte';
	import TaskCard from '$lib/components/kanban/TaskCard.svelte';
	import EventTimeline from '$lib/components/kanban/EventTimeline.svelte';
	import type { Board, KanbanEvent, Task, TaskStatus } from '$lib/types/kanban';

	const boardId = $derived(page.params.id ?? '');
	let board = $state<Board | null>(null);
	let tasks = $state<Task[]>([]);
	let events = $state<KanbanEvent[]>([]);
	let error = $state<string | null>(null);
	let sse: SseClient | null = null;

	let creating = $state(false);
	let newTitle = $state('');
	let newBody = $state('');
	let newAssignee = $state('coder');

	const byStatus = $derived<Record<TaskStatus, Task[]>>({
		triage:      tasks.filter(t => t.status === 'triage'),
		ready:       tasks.filter(t => t.status === 'ready'),
		in_progress: tasks.filter(t => t.status === 'in_progress'),
		done:        tasks.filter(t => t.status === 'done'),
		blocked:     tasks.filter(t => t.status === 'blocked'),
		cancelled:   tasks.filter(t => t.status === 'cancelled'),
		failed:      tasks.filter(t => t.status === 'failed')
	});
	const counts = $derived<Record<TaskStatus, number>>({
		triage:      byStatus.triage.length,
		ready:       byStatus.ready.length,
		in_progress: byStatus.in_progress.length,
		done:        byStatus.done.length,
		blocked:     byStatus.blocked.length,
		cancelled:   byStatus.cancelled.length,
		failed:      byStatus.failed.length
	});

	async function refresh() {
		error = null;
		try {
			board = await getBoard(boardId);
			[tasks, events] = await Promise.all([listBoardTasks(boardId), listBoardEvents(boardId)]);
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		}
	}

	async function onCreateTask() {
		if (!newTitle.trim()) return;
		try {
			await createTask(boardId, newTitle.trim(), newBody, newAssignee);
			newTitle = '';
			newBody = '';
			creating = false;
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '创建失败';
		}
	}

	async function onCancel(id: string) {
		if (!confirm('确认取消这个 task?')) return;
		try {
			await cancelTask(id);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '取消失败';
		}
	}

	onMount(() => {
		refresh();
		// SSE 实时刷新
		sse = connectSse('/v1/events', (ev) => {
			const k = parseKanbanEvent(ev.data);
			if (k && 'task_id' in k && tasks.find(t => t.id === k.task_id)) {
				// 该 task 状态变化, refresh
				refresh();
			} else if (k) {
				// 新 task 创建/分配, 追加到 events
				events = [{ id: Date.now(), task_id: 'task_id' in k ? k.task_id : null, run_id: null, kind: k.type, payload: k, created_at: new Date().toISOString() }, ...events].slice(0, 50) as any;
			}
		});
	});

	onDestroy(() => {
		sse?.close();
	});
</script>

<svelte:head>
	<title>{board?.name ?? 'Board'} · 千寻</title>
</svelte:head>

<div class="flex h-full flex-col gap-3 p-4">
	<!-- header -->
	<header class="flex items-center justify-between">
		<div class="flex items-center gap-2">
			<a href="/kanban" class="hover:bg-accent rounded p-1" data-testid="back-link">
				<ArrowLeft class="size-4" />
			</a>
			<h1 class="text-lg font-semibold">{board?.name ?? '加载中...'}</h1>
			<span class="text-muted-foreground text-xs">({tasks.length} tasks)</span>
		</div>
		<button
			type="button"
			class="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex items-center gap-1 rounded px-3 py-1 text-sm"
			onclick={() => (creating = !creating)}
			data-testid="new-task-btn"
		>
			<Plus class="size-3" />
			新建 Task
		</button>
	</header>

	{#if error}
		<div class="rounded border border-red-300 bg-red-50 p-2 text-xs text-red-700" data-testid="error">
			{error}
		</div>
	{/if}

	<!-- 新建 task 表单 -->
	{#if creating}
		<div class="bg-card rounded border p-3" data-testid="new-task-form">
			<div class="flex flex-col gap-2 sm:flex-row">
				<input
					type="text"
					bind:value={newTitle}
					placeholder="Task 标题"
					class="flex-1 rounded border bg-background px-2 py-1 text-sm"
					data-testid="new-task-title"
				/>
				<select
					bind:value={newAssignee}
					class="rounded border bg-background px-2 py-1 text-sm"
					data-testid="new-task-role"
				>
					<option value="coder">coder</option>
					<option value="techlead">techlead</option>
					<option value="verifier">verifier</option>
					<option value="researcher">researcher</option>
				</select>
				<button
					type="button"
					class="bg-primary text-primary-foreground rounded px-3 py-1 text-sm disabled:opacity-50"
					onclick={onCreateTask}
					disabled={!newTitle.trim()}
				>
					创建
				</button>
				<button
					type="button"
					class="hover:bg-muted rounded border px-2 py-1"
					onclick={() => (creating = false)}
				>
					<X class="size-3" />
				</button>
			</div>
			<textarea
				bind:value={newBody}
				placeholder="任务描述 (可选)"
				class="mt-2 w-full rounded border bg-background px-2 py-1 text-xs"
				rows="2"
			></textarea>
		</div>
	{/if}

	<!-- 5 列 board 视图 -->
	<div class="grid flex-1 grid-cols-1 gap-3 overflow-x-auto md:grid-cols-3 lg:grid-cols-5">
		{#each Object.entries(byStatus) as [status, items] (status)}
			<section class="bg-card/50 flex min-w-[200px] flex-col gap-2 rounded border p-2" data-testid={`col-${status}`}>
				<header class="flex items-center justify-between">
					<StatusBadge status={status as TaskStatus} />
					<span class="text-muted-foreground text-xs">({counts[status as TaskStatus]})</span>
				</header>
				<div class="flex flex-col gap-2">
					{#each items as t (t.id)}
						<div class="relative">
							<TaskCard task={t} />
							{#if status !== 'done' && status !== 'cancelled'}
								<button
									type="button"
									class="hover:bg-destructive/10 absolute top-1 right-1 rounded p-0.5 opacity-0 transition-opacity group-hover:opacity-100"
									onclick={(e) => { e.stopPropagation(); onCancel(t.id); }}
									title="取消"
									data-testid="cancel-{t.id}"
								>
									<X class="size-3" />
								</button>
							{/if}
						</div>
					{/each}
					{#if items.length === 0}
						<div class="text-muted-foreground py-2 text-center text-[10px]">空</div>
					{/if}
				</div>
			</section>
		{/each}
	</div>

	<!-- 事件时间线 (固定底部 30vh) -->
	<div class="h-[30vh] shrink-0">
		<h3 class="mb-1 text-xs font-semibold">事件时间线 (SSE 实时)</h3>
		<EventTimeline events={events} max={30} />
	</div>
</div>
