<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import { projectStore } from '$lib/stores/project.svelte';
	import ProjectItem from './ProjectItem.svelte';

	// 2026-06-09: projectStore 改用 listSessions('all') 按 project_root 去重 derive Project[].
	// 不再依赖 buildSeed() mock. 数据源是真实 SQLite (daemon.db).
	const projects = $derived(projectStore.all);
	const totalSessions = $derived(projectStore.all.reduce((sum, p) => sum + p.session_count, 0));
</script>

<div>
	<div class="px-3 py-1 text-[11px] text-zinc-500 font-medium tracking-wide flex items-center justify-between">
		<span>项目 ({totalSessions})</span>
		<button class="text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300" aria-label="新建项目">
			<Icon name="plus" class="w-3 h-3" />
		</button>
	</div>
	<div class="px-1.5 space-y-0.5">
		{#each projects as p (p.id)}
			<ProjectItem project={p} />
		{/each}
		{#if projects.length === 0}
			<p class="px-3 py-2 text-[11px] text-zinc-400 italic">还没有项目. 发送第一条消息后自动 derive.</p>
		{/if}
	</div>
</div>
