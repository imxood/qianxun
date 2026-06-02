<script lang="ts">
	import type { Project, Team } from "$lib/types/ipc";
	import Folder from "@lucide/svelte/icons/folder";
	import Plus from "@lucide/svelte/icons/plus";
	import TeamSwitcher from "$lib/components/team/TeamSwitcher.svelte";
	import MemberList from "$lib/components/team/MemberList.svelte";

	type Props = {
		projects: Project[];
		teams: Team[];
		activeProjectId?: string;
		activeTeamId?: string;
		activeMemberId?: string;
		onSelectProject?: (id: string) => void;
		onSelectTeam?: (id: string) => void;
	};

	let {
		projects,
		teams,
		activeProjectId,
		activeTeamId,
		activeMemberId,
		onSelectProject,
		onSelectTeam,
	}: Props = $props();

	/// 派生: 当前 active team (用于 MemberList)
	const activeTeam = $derived(teams.find((t) => t.id === activeTeamId) ?? teams[0]);
</script>

<section class="flex h-full flex-col gap-4">
	<!-- 团队切换器 (顶部) -->
	<TeamSwitcher
		{teams}
		{activeTeamId}
		onSelect={(id) => onSelectTeam?.(id)}
	/>

	<!-- 项目列表 (中段, 弹性) -->
	<header class="flex min-h-0 flex-1 flex-col">
		<div class="mb-1 flex items-center justify-between">
			<div
				class="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
			>
				<Folder class="size-3" />
				<span>项目</span>
			</div>
			<button
				class="text-muted-foreground hover:text-foreground"
				title="新建项目 (Stage 6)"
				aria-label="新建项目"
			>
				<Plus class="size-3.5" />
			</button>
		</div>
		<div class="flex flex-col gap-0.5 overflow-y-auto">
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
		</div>
	</header>

	<!-- 成员列表 (底部, 折叠) -->
	{#if activeTeam}
		<MemberList members={activeTeam.members} {activeMemberId} />
	{/if}
</section>
