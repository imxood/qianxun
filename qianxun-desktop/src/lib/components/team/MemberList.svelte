<!--
  MemberList.svelte — Stage 5 §9 成员列表
  与 docs/30_子项目规划/03-tauri-desktop.md §9.2 一致
  渲染模型:
    - 折叠面板 (可展开/收起)
    - 展开后: 头像/首字母 + display_name + role badge
    - 当前用户高亮 (可选, 现阶段不强制)

  Stage 5 简化:
    - 只读 (角色编辑留 Stage 6 管理员写操作)
    - 头像用首字母圆形 fallback (Stage 6 接 avatar_url)
-->
<script lang="ts">
	import type { TeamMember, TeamRole } from "$lib/types/ipc";
	import ChevronDown from "@lucide/svelte/icons/chevron-down";
	import ChevronRight from "@lucide/svelte/icons/chevron-right";

	type Props = {
		members: TeamMember[];
		activeMemberId?: string;
	};

	let { members, activeMemberId }: Props = $props();

	let expanded = $state<boolean>(false);

	function toggle(): void {
		expanded = !expanded;
	}

	function initials(name: string): string {
		const parts = name.trim().split(/\s+/);
		const first = parts[0]?.[0] ?? "?";
		const last = parts.length > 1 ? parts[parts.length - 1]?.[0] ?? "" : "";
		return (first + last).toUpperCase();
	}

	function roleLabel(role: TeamRole): string {
		switch (role) {
			case "owner":
				return "所有者";
			case "admin":
				return "管理员";
			case "developer":
				return "开发者";
			case "viewer":
				return "观察者";
		}
	}

	function roleColor(role: TeamRole): string {
		switch (role) {
			case "owner":
				return "bg-purple-100 text-purple-900 dark:bg-purple-950 dark:text-purple-200";
			case "admin":
				return "bg-blue-100 text-blue-900 dark:bg-blue-950 dark:text-blue-200";
			case "developer":
				return "bg-emerald-100 text-emerald-900 dark:bg-emerald-950 dark:text-emerald-200";
			case "viewer":
				return "bg-zinc-100 text-zinc-700 dark:bg-zinc-800 dark:text-zinc-300";
		}
	}
</script>

<div class="flex flex-col gap-1">
	<button
		type="button"
		class="flex w-full items-center justify-between rounded px-1.5 py-1 text-left text-xs font-semibold uppercase tracking-wide text-muted-foreground hover:bg-accent hover:text-accent-foreground"
		onclick={toggle}
		aria-expanded={expanded}
	>
		<span class="flex items-center gap-1.5">
			{#if expanded}
				<ChevronDown class="size-3" />
			{:else}
				<ChevronRight class="size-3" />
			{/if}
			<span>成员 ({members.length})</span>
		</span>
	</button>

	{#if expanded}
		<ul class="flex flex-col gap-1">
			{#each members as member (member.user_id)}
				<li
					class="flex items-center gap-2 rounded px-1.5 py-1 text-sm hover:bg-accent"
					class:bg-accent={member.user_id === activeMemberId}
				>
					<div
						class="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/10 text-[10px] font-semibold text-primary"
					>
						{initials(member.display_name)}
					</div>
					<div class="min-w-0 flex-1 truncate">
						<div class="truncate text-sm font-medium">{member.display_name}</div>
					</div>
					<span
						class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium {roleColor(member.role)}"
					>
						{roleLabel(member.role)}
					</span>
				</li>
			{/each}
			{#if members.length === 0}
				<li class="px-2 py-1 text-xs text-muted-foreground">暂无成员</li>
			{/if}
		</ul>
	{/if}
</div>
