// ───────────────────────────────────────────────────────────────────────────
// TeamStore — Stage 6c 团队状态 (替代 Stage 6b vpsStore 的本地 mock 状态)
//
// 职责:
//   - 持有 teams / activeTeamId / members / projects / assignments
//   - activeTeam 派生当前 team
//   - refresh() 从 VPS 拉 teams + members + projects (真接 fetch)
//   - seed() 临时: 用 mock 数据灌入 (Stage 7 替换为纯 VPS 拉取)
//
// 数据流:
//   写操作 (inviteMember / changeRole / assignProject) → vpsStore
//     组件 onChanged 回调 → teamStore.refresh() → 重新拉数据
//
// 约束:
//   - 不做乐观更新 (Stage 7 考虑)
//   - 不做实时 WS 同步 (Stage 7)
//   - assignments 仅在 refresh() 一次性拉 (Stage 7 改为 per-team 增量)
// ───────────────────────────────────────────────────────────────────────────

import type { Project, Team, TeamMember } from "$lib/types/ipc";
import { settingsStore } from "$lib/stores/settings.svelte";

class TeamStore {
	teams = $state<Team[]>([]);
	activeTeamId = $state<string | null>(null);
	members = $state<TeamMember[]>([]);
	projects = $state<Project[]>([]);
	/// project_id → user_id[]  (Stage 6c: refresh 时由后端拉, 写操作后也重拉)
	assignments = $state<Record<string, string[]>>({});
	loading = $state<boolean>(false);
	lastError = $state<string | null>(null);

	// ─── 派生 ────────────────────────────────────────────────────────────────

	get activeTeam(): Team | null {
		return this.teams.find((t) => t.id === this.activeTeamId) ?? null;
	}

	// ─── 切换 / 设置 ─────────────────────────────────────────────────────────

	setActiveTeam(teamId: string): void {
		this.activeTeamId = teamId;
		this._syncActiveMembers();
	}

	/// Stage 6c 临时: +page.svelte 启动时用 mock 灌入, 让 UI 立即可见.
	/// Stage 7 替换为 refresh() 真实拉取 (VPS 在线时).
	seed(teams: Team[], projects: Project[]): void {
		this.teams = teams;
		this.projects = projects;
		if (teams.length > 0 && !this.activeTeamId) {
			this.activeTeamId = teams[0]?.id ?? null;
		}
		this._syncActiveMembers();
	}

	// ─── Stage 6c: refresh (真接 fetch) ─────────────────────────────────────

	/// 从 VPS 拉 teams + projects. members 来自 activeTeam.members.
	/// 若 VPS 未配 URL 或拉取失败 → 静默保留已有状态 (不抛, 让 UI 不闪).
	async refresh(): Promise<void> {
		const base = settingsStore.vpsUrl.trim();
		if (!base) {
			// VPS 未配置: 静默 noop, 等用户去 Settings 配
			return;
		}
		this.loading = true;
		this.lastError = null;
		try {
			const [teamsRes, projectsRes] = await Promise.all([
				vpsGet<{ teams: Team[] }>("/api/teams"),
				vpsGet<{ projects: Project[] }>("/api/projects"),
			]);
			this.teams = teamsRes.teams;
			this.projects = projectsRes.projects;
			// activeTeamId 仍指向某 team: 保留 (若该 team 不在新列表中, 切到第一个)
			if (
				!this.activeTeamId ||
				!this.teams.some((t) => t.id === this.activeTeamId)
			) {
				this.activeTeamId = this.teams[0]?.id ?? null;
			}
			this._syncActiveMembers();
		} catch (e) {
			this.lastError = (e as Error).message || "refresh teams/projects 失败";
		} finally {
			this.loading = false;
		}
	}

	// ─── 内部 ────────────────────────────────────────────────────────────────

	_syncActiveMembers(): void {
		const team = this.activeTeam;
		this.members = team ? [...team.members] : [];
	}
}

/// 通用 GET helper (与 vps.svelte.ts 内的 vpsFetch 行为一致, 不带 body).
async function vpsGet<T>(path: string): Promise<T> {
	const base = settingsStore.vpsUrl.trim();
	if (!base) {
		throw new Error("vpsGet: settingsStore.vpsUrl 未配置");
	}
	const token = settingsStore.getVpsToken();
	const r = await fetch(`${base}${path}`, {
		method: "GET",
		headers: {
			Authorization: token ? `Bearer ${token}` : "",
			"Content-Type": "application/json",
		},
	});
	if (!r.ok) {
		throw new Error(`HTTP ${r.status} ${r.statusText}`);
	}
	return (await r.json()) as T;
}

export const teamStore = new TeamStore();
