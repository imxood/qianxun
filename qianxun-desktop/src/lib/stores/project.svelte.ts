// qianxun-desktop/src/lib/stores/project.svelte.ts
// Project store
//
// Stage 4a (sub-task #4): 删 buildSeed 依赖.
//   - 后端 RuntimeApi 暂没 project CRUD 方法 (sub-task #3 范围之外)
//   - 业务: project 由用户手动建 (绑工作目录), 后续 sub-task 接 RuntimeApi
//   - 当前: 空数组 + loadAll() 异步方法 (留接口, 暂时返空)
//
// 关联:
//   - docs/30_子项目规划/04b-tauri-runtime-integration.md §"Sub-task 6"
//   - qianxun-runtime/src/api/trait_def.rs (RuntimeApi 5 方法, project 不在内)

import type { Project } from '$lib/types/entity';

function createProjectStore() {
	const projects = $state<Project[]>([]);
	let initialized = $state(false);
	let loading = $state(false);
	let lastError = $state<string | null>(null);

	/// 启动时调. 当前 noop, 后续 sub-task 接 RuntimeApi 拉真实 project 列表.
	/// 重复调用安全.
	async function loadAll() {
		if (initialized || loading) return;
		loading = true;
		lastError = null;
		try {
			// TODO: 等后端 RuntimeApi 加 list_projects / create_project
			// const r = await listProjects();
			// projects.push(...r.projects);
			initialized = true;
		} catch (e) {
			lastError = (e as Error).message ?? String(e);
		} finally {
			loading = false;
		}
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
