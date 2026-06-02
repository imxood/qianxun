<!--
  MemberEditor.svelte — Stage 6b 写操作 UI
  与 docs/30_子项目规划/03-tauri-desktop.md §9.3 一致
  在 Stage 5 MemberList (只读) 基础上加:
    - 每行: role 下拉, 改动 → vpsStore.changeRole(...)
    - 末尾: "+ 加成员" 按钮 → 弹输入 user_id + role → vpsStore.inviteMember(...)
    - 阶段 banner: 写操作当前为本地 mock (Stage 6c 接入真实 fetch)

  约束 (Stage 6b 简化):
    - 写操作不真发请求, 只更新本地 teamMembers 状态 (见 vps.svelte.ts)
    - 成员删/转让 owner 留 Stage 7
-->
<script lang="ts">
	import type { TeamMember, TeamRole } from "$lib/types/ipc";
	import { vpsStore } from "$lib/stores/vps.svelte";
	import Plus from "@lucide/svelte/icons/plus";
	import Check from "@lucide/svelte/icons/check";
	import X from "@lucide/svelte/icons/x";

	type Props = {
		teamId: string;
		members: TeamMember[];
		onChanged?: () => void;
	};

	let { teamId, members, onChanged }: Props = $props();

	let adding = $state<boolean>(false);
	let newUserId = $state<string>("");
	let newDisplayName = $state<string>("");
	let newRole = $state<TeamRole>("developer");
	let submitting = $state<boolean>(false);
	let lastError = $state<string | null>(null);

	// 仅当 user_id 非空且未在列表中时, 启用提交
	const canSubmit = $derived(
		!submitting && newUserId.trim().length > 0 && !members.some((m) => m.user_id === newUserId.trim())
	);

	async function invite(): Promise<void> {
		if (!canSubmit) return;
		submitting = true;
		lastError = null;
		try {
			await vpsStore.inviteMember(teamId, newUserId.trim(), newDisplayName.trim() || newUserId.trim(), newRole);
			// 重置表单
			newUserId = "";
			newDisplayName = "";
			newRole = "developer";
			adding = false;
			onChanged?.();
		} catch (e) {
			lastError = (e as Error).message || "邀请失败";
		} finally {
			submitting = false;
		}
	}

	async function changeRole(userId: string, role: TeamRole): Promise<void> {
		try {
			await vpsStore.changeRole(teamId, userId, role);
			onChanged?.();
		} catch (e) {
			lastError = (e as Error).message || "改角色失败";
		}
	}

	function cancelAdd(): void {
		adding = false;
		newUserId = "";
		newDisplayName = "";
		newRole = "developer";
		lastError = null;
	}
</script>

<div class="flex flex-col gap-1">
	<div
		class="flex items-center justify-between rounded px-1.5 py-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground"
	>
		<span>成员管理 ({members.length})</span>
		{#if !adding}
			<button
				type="button"
				class="flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground hover:bg-accent hover:text-accent-foreground"
				onclick={() => (adding = true)}
				title="加成员 (Stage 6b mock)"
				aria-label="加成员"
			>
				<Plus class="size-3" />
				<span>加成员</span>
			</button>
		{/if}
	</div>

	<!-- 已有成员: 角色下拉 -->
	<ul class="flex flex-col gap-1">
		{#each members as member (member.user_id)}
			<li class="flex items-center gap-2 rounded px-1.5 py-1 text-sm hover:bg-accent">
				<div
					class="flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/10 text-[10px] font-semibold text-primary"
				>
					{(member.display_name[0] ?? "?").toUpperCase()}
				</div>
				<div class="min-w-0 flex-1 truncate text-sm font-medium">
					{member.display_name}
				</div>
				<select
					value={member.role}
					onchange={(e) => void changeRole(member.user_id, e.currentTarget.value as TeamRole)}
					class="rounded border border-border bg-background px-1 py-0.5 text-[10px] shadow-sm focus:border-ring focus:outline-none focus:ring-1 focus:ring-ring"
					aria-label="角色"
				>
					<option value="owner">owner</option>
					<option value="admin">admin</option>
					<option value="developer">developer</option>
					<option value="viewer">viewer</option>
				</select>
			</li>
		{/each}
		{#if members.length === 0}
			<li class="px-2 py-1 text-xs text-muted-foreground">暂无成员</li>
		{/if}
	</ul>

	<!-- 加成员表单 -->
	{#if adding}
		<div class="flex flex-col gap-1.5 rounded border border-border bg-muted/30 p-2">
			<div class="flex items-center gap-1.5">
				<input
					type="text"
					bind:value={newUserId}
					placeholder="user_id (e.g. u_alice)"
					class="min-w-0 flex-1 rounded border border-border bg-background px-1.5 py-0.5 text-xs focus:border-ring focus:outline-none focus:ring-1 focus:ring-ring"
					aria-label="新成员 user_id"
				/>
				<select
					bind:value={newRole}
					class="rounded border border-border bg-background px-1 py-0.5 text-xs shadow-sm focus:border-ring focus:outline-none focus:ring-1 focus:ring-ring"
					aria-label="新成员角色"
				>
					<option value="admin">admin</option>
					<option value="developer">developer</option>
					<option value="viewer">viewer</option>
				</select>
			</div>
			<input
				type="text"
				bind:value={newDisplayName}
				placeholder="显示名 (可选, 留空用 user_id)"
				class="rounded border border-border bg-background px-1.5 py-0.5 text-xs focus:border-ring focus:outline-none focus:ring-1 focus:ring-ring"
				aria-label="新成员显示名"
			/>
			<div class="flex items-center justify-end gap-1.5">
				<button
					type="button"
					class="flex items-center gap-1 rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-accent"
					onclick={cancelAdd}
					aria-label="取消"
				>
					<X class="size-3" />
					<span>取消</span>
				</button>
				<button
					type="button"
					class="flex items-center gap-1 rounded bg-primary px-1.5 py-0.5 text-xs font-medium text-primary-foreground disabled:opacity-50"
					onclick={() => void invite()}
					disabled={!canSubmit}
					aria-label="确认加成员"
				>
					<Check class="size-3" />
					<span>{submitting ? "提交中…" : "确认"}</span>
				</button>
			</div>
			{#if lastError}
				<div class="text-[10px] text-red-600 dark:text-red-400">{lastError}</div>
			{/if}
		</div>
	{/if}

</div>
