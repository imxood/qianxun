<!--
  ProjectAssignPanel.svelte — Stage 6b 项目分配面板
  与 docs/30_子项目规划/03-tauri-desktop.md §9.4 一致
  在 Stage 6b 加一个独立小组件:
    - 上半: 项目 id/name + "已分配 N 人" 计数
    - 中间: 已分配成员 chips (可移除, 留 Stage 7)
    - 下半: 选一个未分配的 team 成员 + "分配" 按钮
    - banner: 写操作当前为本地 mock (Stage 6c 接入真实 fetch)

  约束 (Stage 6b 简化):
    - 不做移除 (Stage 7)
    - 选成员后只能分配给本组件绑定的 project, 不能跨项目
-->
<script lang="ts">
	import type { TeamMember, Project } from "$lib/types/ipc";
	import { vpsStore } from "$lib/stores/vps.svelte";
	import UserPlus from "@lucide/svelte/icons/user-plus";
	import Users from "@lucide/svelte/icons/users";

	type Props = {
		project: Project;
		members: TeamMember[];
		assignees?: string[];
		onChanged?: () => void;
	};

	let { project, members, assignees = [], onChanged }: Props = $props();

	let selectedUserId = $state<string>("");
	let submitting = $state<boolean>(false);
	let lastError = $state<string | null>(null);

	// 派生: 未分配的成员 (下拉候选)
	const unassigned = $derived(
		members.filter((m) => !assignees.includes(m.user_id))
	);

	// 派生: 已分配的成员 (用完整对象渲染)
	const assignedMembers = $derived(
		assignees
			.map((id) => members.find((m) => m.user_id === id))
			.filter((m): m is TeamMember => Boolean(m))
	);

	async function assign(): Promise<void> {
		if (!selectedUserId || submitting) return;
		submitting = true;
		lastError = null;
		try {
			await vpsStore.assignProject(project.id, selectedUserId);
			selectedUserId = "";
			onChanged?.();
		} catch (e) {
			lastError = (e as Error).message || "分配失败";
		} finally {
			submitting = false;
		}
	}
</script>

<div class="flex flex-col gap-2 rounded border border-border bg-card p-2">
	<header class="flex items-center gap-1.5">
		<Users class="size-3.5 text-muted-foreground" />
		<div class="min-w-0 flex-1">
			<div class="truncate text-sm font-medium">{project.name}</div>
			<div class="truncate text-[10px] text-muted-foreground">{project.id}</div>
		</div>
		<span class="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
			已分配 {assignees.length}
		</span>
	</header>

	<!-- 已分配成员 chips -->
	{#if assignedMembers.length > 0}
		<div class="flex flex-wrap gap-1">
			{#each assignedMembers as m (m.user_id)}
				<span
					class="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary"
					title={`${m.display_name} (${m.role})`}
				>
					{m.display_name}
				</span>
			{/each}
		</div>
	{:else}
		<div class="text-[10px] text-muted-foreground">暂无已分配成员</div>
	{/if}

	<!-- 分配新成员 -->
	<div class="flex items-center gap-1.5">
		<select
			bind:value={selectedUserId}
			class="min-w-0 flex-1 rounded border border-border bg-background px-1.5 py-1 text-xs shadow-sm focus:border-ring focus:outline-none focus:ring-1 focus:ring-ring"
			aria-label="选择团队成员"
			disabled={unassigned.length === 0}
		>
			<option value="">
				{unassigned.length === 0 ? "(全部成员已分配)" : "选择成员…"}
			</option>
			{#each unassigned as m (m.user_id)}
				<option value={m.user_id}>
					{m.display_name} ({m.role})
				</option>
			{/each}
		</select>
		<button
			type="button"
			class="flex items-center gap-1 rounded bg-primary px-2 py-1 text-xs font-medium text-primary-foreground disabled:opacity-50"
			onclick={() => void assign()}
			disabled={!selectedUserId || submitting}
			aria-label="分配成员"
		>
			<UserPlus class="size-3" />
			<span>{submitting ? "提交中…" : "分配"}</span>
		</button>
	</div>

	{#if lastError}
		<div class="text-[10px] text-red-600 dark:text-red-400">{lastError}</div>
	{/if}
</div>
