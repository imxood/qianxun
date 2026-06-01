<script lang="ts">
	import type { Project, Team } from "$lib/types/ipc";
	import Folder from "@lucide/svelte/icons/folder";
	import Users from "@lucide/svelte/icons/users";
	import Plus from "@lucide/svelte/icons/plus";

	type Props = {
		projects: Project[];
		teams: Team[];
		activeProjectId?: string;
		onSelectProject?: (id: string) => void;
	};

	let { projects, teams, activeProjectId, onSelectProject }: Props = $props();
</script>

<section class="flex flex-col gap-4">
	<header>
		<div class="mb-1 flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
			<Users class="size-3" />
			<span>团队</span>
		</div>
		{#each teams as team (team.id)}
			<div
				class="cursor-pointer rounded px-2 py-1 text-sm hover:bg-accent hover:text-accent-foreground"
				class:bg-accent={activeProjectId && teams[0]?.id === team.id}
			>
				{team.name}
				<span class="ml-1 text-xs text-muted-foreground">({team.members.length})</span>
			</div>
		{/each}
		{#if teams.length === 0}
			<div class="px-2 py-1 text-xs text-muted-foreground">暂无团队</div>
		{/if}
	</header>

	<header>
		<div class="mb-1 flex items-center justify-between">
			<div class="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
				<Folder class="size-3" />
				<span>项目</span>
			</div>
			<button
				class="text-muted-foreground hover:text-foreground"
				title="新建项目 (Stage 2)"
				aria-label="新建项目"
			>
				<Plus class="size-3.5" />
			</button>
		</div>
		{#each projects as project (project.id)}
			<button
				type="button"
				class="block w-full cursor-pointer rounded px-2 py-1 text-left text-sm hover:bg-accent hover:text-accent-foreground"
				class:bg-accent={project.id === activeProjectId}
				onclick={() => onSelectProject?.(project.id)}
			>
				<div class="truncate font-medium">{project.name}</div>
				<div class="truncate text-xs text-muted-foreground">{project.path}</div>
			</button>
		{/each}
		{#if projects.length === 0}
			<div class="px-2 py-1 text-xs text-muted-foreground">暂无项目</div>
		{/if}
	</header>
</section>
