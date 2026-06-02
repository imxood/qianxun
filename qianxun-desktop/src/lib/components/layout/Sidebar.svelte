<!--
  Sidebar.svelte — Stage 6c 重构
  与 docs/30_子项目规划/03-tauri-desktop.md §9 一致
  集成:
    - TeamSwitcher (顶部, 切换 active team)
    - MemberList   (只读, 当前 team 成员)
    - MemberEditor (写操作, 加成员/改角色)  ← 新增
    - ProjectList  (中段)
    - ProjectAssignPanel (每个项目 1 个)   ← 新增
  状态源: teamStore (Stage 6c 新建, 替代 props 传递)
  写操作回调: 完成后调 teamStore.refresh() 拉新
-->
<script lang="ts">
	import Folder from "@lucide/svelte/icons/folder";
	import Plus from "@lucide/svelte/icons/plus";
	import TeamSwitcher from "$lib/components/team/TeamSwitcher.svelte";
	import MemberList from "$lib/components/team/MemberList.svelte";
	import MemberEditor from "$lib/components/team/MemberEditor.svelte";
	import ProjectAssignPanel from "$lib/components/team/ProjectAssignPanel.svelte";
	import { teamStore } from "$lib/stores/team.svelte";

	type Props = {
		activeProjectId?: string;
		onSelectProject?: (id: string) => void;
	};

	let { activeProjectId, onSelectProject }: Props = $props();
</script>

<section class="flex h-full flex-col gap-3">
	<!-- 团队切换器 (顶部) -->
	<TeamSwitcher
		teams={teamStore.teams}
		activeTeamId={teamStore.activeTeamId ?? undefined}
		onSelect={(id) => teamStore.setActiveTeam(id)}
	/>

	<!-- 成员列表 + 编辑器 (中上, 仅当有 active team) -->
	{#if teamStore.activeTeam}
		<MemberList
			members={teamStore.members}
			activeMemberId={undefined}
		/>
		<MemberEditor
			teamId={teamStore.activeTeam.id}
			members={teamStore.members}
			onChanged={() => void teamStore.refresh()}
		/>
	{/if}

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
				title="新建项目 (Stage 7)"
				aria-label="新建项目"
				disabled
			>
				<Plus class="size-3.5" />
			</button>
		</div>
		<div class="flex flex-col gap-0.5 overflow-y-auto">
			{#each teamStore.projects as project (project.id)}
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
			{#if teamStore.projects.length === 0}
				<div class="px-2 py-1 text-xs text-muted-foreground">暂无项目</div>
			{/if}
		</div>
	</header>

	<!-- 项目分配面板 (底部) — 每个 project 一个 -->
	{#if teamStore.projects.length > 0 && teamStore.activeTeam}
		<div class="flex flex-col gap-2">
			{#each teamStore.projects as project (project.id)}
				<ProjectAssignPanel
					{project}
					members={teamStore.members}
					assignees={teamStore.assignments[project.id] ?? []}
					onChanged={() => void teamStore.refresh()}
				/>
			{/each}
		</div>
	{/if}
</section>
