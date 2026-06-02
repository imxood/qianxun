<!--
  TeamSwitcher.svelte — Stage 5 §9 团队切换器
  与 docs/30_子项目规划/03-tauri-desktop.md §9.1 一致
  渲染模型:
    - 左栏顶部下拉选单 (写操作在 Stage 6, 这里只读 + onSelect 回调)
    - 显示团队名 + 成员数
    - 空态: '暂无团队' 占位

  Stage 5 简化:
    - 不做 "+ 新建团队" 按钮 (写操作留 Stage 6)
    - 不做拖拽排序
-->
<script lang="ts">
	import type { Team } from "$lib/types/ipc";
	import Users from "@lucide/svelte/icons/users";
	import ChevronDown from "@lucide/svelte/icons/chevron-down";

	type Props = {
		teams: Team[];
		activeTeamId?: string;
		onSelect: (teamId: string) => void;
	};

	let { teams, activeTeamId, onSelect }: Props = $props();
</script>

<div class="flex flex-col gap-1">
	<div class="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
		<Users class="size-3" />
		<span>团队</span>
	</div>

	{#if teams.length === 0}
		<div class="px-2 py-1 text-xs text-muted-foreground">暂无团队</div>
	{:else}
		<div class="relative">
			<select
				value={activeTeamId ?? teams[0]?.id ?? ""}
				onchange={(e) => onSelect(e.currentTarget.value)}
				class="w-full appearance-none rounded border border-border bg-background px-2 py-1.5 pr-7 text-sm font-medium shadow-sm focus:border-ring focus:outline-none focus:ring-1 focus:ring-ring"
			>
				{#each teams as team (team.id)}
					<option value={team.id}>
						{team.name} ({team.members.length} 成员)
					</option>
				{/each}
			</select>
			<ChevronDown
				class="pointer-events-none absolute right-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
			/>
		</div>
	{/if}
</div>
