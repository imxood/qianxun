<script lang="ts">
	// /kanban — boards 列表 + 创建 (2026-06-04 阶段 3, MVP-3 落地)
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { Plus, KanbanSquare, Folder } from '@lucide/svelte';
	import { listBoards, createBoard } from '$lib/api/kanban';
	import { listProjects } from '$lib/api/projects';
	import { authStore } from '$lib/stores/auth.svelte';
	import type { Board, Project } from '$lib/types/kanban';

	let boards = $state<Board[]>([]);
	let projects = $state<Project[]>([]);
	let error = $state<string | null>(null);
	let creating = $state(false);
	let newName = $state('');
	let newProjectRoot = $state('');

	async function refresh() {
		error = null;
		try {
			[boards, projects] = await Promise.all([listBoards(), listProjects()]);
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		}
	}

	async function onCreate() {
		if (!newName.trim() || !newProjectRoot.trim()) return;
		creating = true;
		try {
			const b = await createBoard(newName.trim(), newProjectRoot.trim());
			// 2026-06-04 fix: 见 routes/+page.svelte 注释 — SvelteKit 2 `goto` 在
			// `paths.base='/ui'` 下要带 base 前缀.
			await goto(`/ui/kanban/${b.id}`);
		} catch (e) {
			error = e instanceof Error ? e.message : '创建失败';
			creating = false;
		}
	}

	function projectName(id: string): string {
		return projects.find(p => p.id === id)?.name ?? id.slice(0, 12);
	}

	onMount(refresh);

	// 2026-06-04 fix: 登录后自动重 fetch (见 llm/+page.svelte 注释)
	let firstRun = true;
	$effect(() => {
		const token = authStore.token;
		if (firstRun) {
			firstRun = false;
			return;
		}
		if (token) {
			void refresh();
		}
	});
</script>

<svelte:head>
	<title>Kanban · 千寻</title>
</svelte:head>

<div class="flex flex-col gap-4 p-6">
	<header class="flex items-center justify-between">
		<div class="flex items-center gap-2">
			<KanbanSquare class="size-5" />
			<h1 class="text-lg font-semibold">Kanban Boards</h1>
			<span class="text-muted-foreground text-xs">({boards.length})</span>
		</div>
	</header>

	{#if error}
		<div class="rounded border border-red-300 bg-red-50 p-3 text-xs text-red-700 dark:border-red-900 dark:bg-red-950 dark:text-red-300" data-testid="error">
			{error}
		</div>
	{/if}

	<!-- 创建表单 -->
	<div class="rounded border bg-card p-3" data-testid="create-form">
		<h3 class="mb-2 text-sm font-semibold">+ 新建 Board</h3>
		<div class="flex flex-col gap-2 sm:flex-row">
			<input
				type="text"
				bind:value={newName}
				placeholder="Board 名称 (e.g. 千寻重构)"
				class="flex-1 rounded border bg-background px-2 py-1 text-sm"
				data-testid="create-name"
			/>
			<input
				type="text"
				bind:value={newProjectRoot}
					placeholder="项目根路径 (e.g. C:/work/qianxun)"
				class="flex-1 rounded border bg-background px-2 py-1 text-sm"
				data-testid="create-root"
			/>
			<button
				type="button"
				class="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex items-center gap-1 rounded px-3 py-1 text-sm disabled:opacity-50"
				onclick={onCreate}
				disabled={creating || !newName.trim() || !newProjectRoot.trim()}
				data-testid="create-submit"
			>
				<Plus class="size-3" />
				{creating ? '创建中...' : '创建'}
			</button>
		</div>
	</div>

	<!-- boards 列表 -->
	{#if boards.length === 0}
		<div class="text-muted-foreground rounded border bg-card p-12 text-center text-sm" data-testid="empty">
			暂无 board. 创建第一个开始
		</div>
	{:else}
		<div class="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3" data-testid="boards-grid">
			{#each boards as b (b.id)}
				<a
					href="/ui/kanban/{b.id}"
					class="hover:bg-accent/30 block rounded border bg-card p-4 transition-colors"
					data-testid="board-card-{b.id}"
				>
					<div class="mb-1 flex items-center gap-2">
						<KanbanSquare class="text-muted-foreground size-4" />
						<h3 class="truncate font-semibold">{b.name}</h3>
					</div>
					<div class="text-muted-foreground flex items-center gap-2 text-xs">
						<Folder class="size-3" />
						<span>{projectName(b.project_id)}</span>
					</div>
					<div class="text-muted-foreground mt-2 font-mono text-[10px]">
						id: {b.id} · default: {b.default_role}
					</div>
					<div class="text-muted-foreground mt-1 text-[10px]">
						{new Date(b.created_at).toLocaleString()}
					</div>
				</a>
			{/each}
		</div>
	{/if}
</div>
