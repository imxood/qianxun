// qianxun-desktop/src/lib/stores/project.svelte.ts
// Project store
//
// 2026-06-09 重构: 之前 loadAll 是 noop + ProjectSection.svelte 用 buildSeed() mock.
// 现状: projectStore.loadAll 调 listSessions('all') 按 SessionInfo.project_root
// 去重, derive 出 Project[]. 零后端改动, 数据源是真实 SQLite (daemon.db).
//
// 未来 (P1 中期): 后端加 RuntimeApi::list_projects / create_project, 支持"先建项目 → 再绑 session"工作流.
// 当前是派生方案, 简单可靠, 不需要新概念.

import type { Project } from '$lib/types/entity';
import { listSessions, type SessionInfo } from '$lib/ipc/runtime';
import { reportError } from '$lib/errors';

function createProjectStore() {
	const projects = $state<Project[]>([]);
	let initialized = $state(false);
	let loading = $state(false);
	let lastError = $state<string | null>(null);

	/// 启动时调. 拉 listSessions('all') 按 project_root 去重, derive Project[].
	/// 重复调用安全.
	async function loadAll() {
		if (initialized || loading) return;
		loading = true;
		lastError = null;
		try {
			const r = await listSessions('all');
			projects.length = 0;
			projects.push(...deriveProjectsFromSessions(r.sessions));
			initialized = true;
		} catch (e) {
			lastError = reportError(e, { source: 'projectStore.loadAll' });
		} finally {
			loading = false;
		}
	}

	/// 从 SessionInfo[] 派生 Project[]. 按 project_root 去重, 统计 session_count.
	/// 没有 project_root 的 session 不计 (顶层 Chat 入口).
	function deriveProjectsFromSessions(sessions: SessionInfo[]): Project[] {
		const byRoot = new Map<string, Project>();
		for (const s of sessions) {
			if (!s.project_root) continue; // 跳过未绑项目的 session
			const existing = byRoot.get(s.project_root);
			if (existing) {
				existing.session_count += 1;
				if (s.last_active_at > existing.last_active_at) {
					existing.last_active_at = s.last_active_at;
				}
			} else {
				byRoot.set(s.project_root, {
					id: s.project_root,
					name: projectNameFromPath(s.project_root),
					session_count: 1,
					created_at: s.created_at,
					last_active_at: s.last_active_at,
				});
			}
		}
		// 按 last_active_at DESC 排序 (最近活跃在前)
		return Array.from(byRoot.values()).sort((a, b) =>
			b.last_active_at.localeCompare(a.last_active_at)
		);
	}

	/// 从绝对路径取最后一段作为项目名 (e.g. /home/maxu/qianxun → "qianxun")
	function projectNameFromPath(path: string): string {
		const m = path.match(/[/\\]([^/\\]+)[/\\]?$/);
		return m ? m[1] : path;
	}

	/// 强制刷新 (绕开 initialized, 跟 create_session 后用). 后续 P1 加.
	async function refresh() {
		initialized = false;
		await loadAll();
	}

	return {
		get all() {
			return projects;
		},
		get initialized() {
			return initialized;
		},
		get loading() {
			return loading;
		},
		get lastError() {
			return lastError;
		},
		get(id: string): Project | undefined {
			return projects.find((p) => p.id === id);
		},
		byId(id: string): Project | undefined {
			return projects.find((p) => p.id === id);
		},
		loadAll,
		refresh,
		/// 测试专用: 重置内部状态. 业务代码不应该调.
		__resetForTesting() {
			projects.length = 0;
			initialized = false;
			loading = false;
			lastError = null;
		},
	};
}

export const projectStore = createProjectStore();
