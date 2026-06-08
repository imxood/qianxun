// qianxun-desktop/src/lib/stores/seed.svelte.ts
// seed / reset actions

import { buildSeed } from '$lib/mock/seed';
import { projectStore } from './project.svelte';
import { sessionStore } from './session.svelte';
import { planStore } from './plan.svelte';
import { subSessionStore } from './sub_session.svelte';
import { browser } from '$app/environment';

function createSeedStore() {
	return {
		resetAll() {
			if (browser) {
				// 清除 localStorage
				const keys = ['projects', 'sessions', 'messages', 'plans', 'subSessions', 'changedFiles', 'ui.col1Width', 'ui.col3Width', 'ui.expandedProjectIds', 'ui.activeView'];
				for (const k of keys) localStorage.removeItem(k);
			}
			// 强制刷新
			location.reload();
		},
		// 测试用: 给一个不刷新版本的 seedAll (调用 init())
		initFromSeed() {
			const seed = buildSeed();
			projectStore; // 触发初始化
			sessionStore; // 触发初始化
		},
	};
}

export const seedStore = createSeedStore();
