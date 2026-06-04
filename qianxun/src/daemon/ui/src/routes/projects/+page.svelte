<script lang="ts">
	// /projects — projects 列表 + 创建 (2026-06-04 阶段 3)
	import { onMount } from 'svelte';
	import { Plus, Folder, Archive } from '@lucide/svelte';
	import { listProjects, createProject } from '$lib/api/projects';
	import { authStore } from '$lib/stores/auth.svelte';
	import type { Project } from '$lib/types/kanban';

	let projects = $state<Project[]>([]);
	let error = $state<string | null>(null);
	let creating = $state(false);
	let newName = $state('');
	let newDesc = $state('');

	async function refresh() {
		error = null;
		try {
			projects = await listProjects();
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		}
	}

	async function onCreate() {
		if (!newName.trim()) return;
		try {
			await createProject(newName.trim(), newDesc);
			newName = '';
			newDesc = '';
			creating = false;
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '创建失败';
		}
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
	<title>Projects · 千寻</title>
</svelte:head>

<div class="flex flex-col gap-4 p-6">
	<header class="flex items-center justify-between">
		<div class="flex items-center gap-2">
			<Folder class="size-5" />
			<h1 class="text-lg font-semibold">Projects</h1>
			<span class="text-muted-foreground text-xs">({projects.length})</span>
		</div>
		<button
			type="button"
			class="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex items-center gap-1 rounded px-3 py-1 text-sm"
			onclick={() => (creating = !creating)}
			data-testid="new-project-btn"
		>
			<Plus class="size-3" />
			新建
		</button>
	</header>

	{#if error}
		<div class="rounded border border-red-300 bg-red-50 p-2 text-xs text-red-700" data-testid="error">
			{error}
		</div>
	{/if}

	{#if creating}
		<div class="bg-card rounded border p-3" data-testid="new-project-form">
			<input
				type="text"
				bind:value={newName}
				placeholder="Project 名称"
				class="mb-2 w-full rounded border bg-background px-2 py-1 text-sm"
				data-testid="new-project-name"
			/>
			<textarea
				bind:value={newDesc}
				placeholder="描述 (可选)"
				class="mb-2 w-full rounded border bg-background px-2 py-1 text-xs"
				rows="2"
			></textarea>
			<button
				type="button"
				class="bg-primary text-primary-foreground rounded px-3 py-1 text-sm disabled:opacity-50"
				onclick={onCreate}
				disabled={!newName.trim()}
			>
				创建
			</button>
		</div>
	{/if}

	{#if projects.length === 0}
		<div class="text-muted-foreground rounded border bg-card p-12 text-center text-sm">
			暂无 project (default project 已自动建)
		</div>
	{:else}
		<div class="grid grid-cols-1 gap-3 md:grid-cols-2" data-testid="projects-grid">
			{#each projects as p (p.id)}
				<div class="rounded border bg-card p-4" data-testid="project-card-{p.id}">
					<div class="mb-1 flex items-center gap-2">
						<Folder class="text-muted-foreground size-4" />
						<h3 class="font-semibold">{p.name}</h3>
						{#if p.status === 'archived'}
							<Archive class="text-muted-foreground size-3" />
						{/if}
					</div>
					{#if p.description}
						<div class="text-muted-foreground mb-2 text-xs">{p.description}</div>
					{/if}
					<div class="text-muted-foreground font-mono text-[10px]">
						id: {p.id} · owner: {p.owner} · status: {p.status}
					</div>
					<div class="text-muted-foreground mt-1 font-mono text-[10px]">
						default_root: {p.default_root || '(empty)'}
					</div>
				</div>
			{/each}
		</div>
	{/if}
</div>
