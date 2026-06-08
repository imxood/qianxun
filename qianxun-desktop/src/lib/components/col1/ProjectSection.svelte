<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import { projectStore } from '$lib/stores/project.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
	import { buildSeed } from '$lib/mock/seed';
	import ProjectItem from './ProjectItem.svelte';

	const projects = $derived(projectStore.all);
	const totalSessions = $derived(buildSeed().sessions.length);
</script>

<div>
	<div class="px-3 py-1 text-[11px] text-zinc-500 font-medium tracking-wide flex items-center justify-between">
		<span>项目</span>
		<button class="text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300" aria-label="新建项目">
			<Icon name="plus" class="w-3 h-3" />
		</button>
	</div>
	<div class="px-1.5 space-y-0.5">
		{#each projects as p (p.id)}
			<ProjectItem project={p} />
		{/each}
	</div>
</div>
