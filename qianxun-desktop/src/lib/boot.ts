// qianxun-desktop/src/lib/boot.ts
// 启动流程 (2026-06-12 桌面端 UI 完整性修复, 从 +page.svelte 抽出).
//
// 4 阶段: listeners → health → lists → done.
// 任一 phase 失败: reportError 上报, 错误累加到 state.error, 不中断后续 phase.
//
// 设计原则 (用户约束 #1 克制 + #3 不要堆小函数):
//   - 数组驱动循环, 1 个循环看清全部启动项
//   - 不拆 4 个 phaseXxx 函数
//   - +page.svelte 从 70 行 bootOnce 减到 5 行调用
//
// 2026-06-12 (批次 2.1): 加 registerUnlisten / cleanupBootListeners.
//   chatStore.init() / planStore.init() 启的 Tauri event listener 句柄
//   之前留在 store 私有, 多次 boot/HMR 后 listener 累积. 改用集中管理:
//   store 调 registerUnlisten(unlistenFn) 把句柄交给 boot, 统一 cleanup.

import { chatStore } from './stores/chat.svelte';
import { planStore } from './stores/plan.svelte';
import { sessionStore } from './stores/session.svelte';
import { projectStore } from './stores/project.svelte';
import { subSessionStore } from './stores/sub_session.svelte';
import { uiStore } from './stores/ui.svelte';
import { reportError } from './errors';

export type BootStep = 'starting' | 'connecting' | 'loading' | 'ready';

export interface BootState {
	step: BootStep;
	progress: number;
	error: string | null;
}

interface BootPhase {
	step: BootStep;
	progress: number;
	label: string;
	run: () => Promise<void>;
}

const _phases: BootPhase[] = [
	{
		step: 'starting',
		progress: 20,
		label: 'listeners',
		run: async () => {
			// store 内部 registerUnlisten 句柄, boot 集中管理.
			await Promise.all([chatStore.init(), planStore.init()]);
		},
	},
	{
		step: 'connecting',
		progress: 50,
		label: 'health',
		run: async () => {
			// 占位: 未来 health_check ping 走这里
		},
	},
	{
		step: 'loading',
		progress: 80,
		label: 'lists',
		run: async () => {
			// 2026-06-12 收尾: subSessionStore.loadAll 拉全量 + 订阅 realtime, 跟
			// sessionStore.init / projectStore.loadAll 同 mode (并行启动, 任一失败
			// reportError 不阻断其它).
			await Promise.all([sessionStore.init(), projectStore.loadAll(), subSessionStore.loadAll()]);
		},
	},
];

// 2026-06-12 (批次 2.1): 集中持有 Tauri event listener 句柄, 避免多次 boot/HMR 累积.
// 业务 store (chat / plan) 调 registerUnlisten 把句柄交过来, cleanupBootListeners 统一释放.
const _unlisteners: Array<() => void> = [];

/** 注册 Tauri event listener 句柄. boot 集中管理, 避免 HMR 多次 boot 累积. */
export function registerUnlisten(fn: () => void): void {
	_unlisteners.push(fn);
}

/** 释放所有已注册 listener 句柄. 测试 (__resetForTesting) / app 卸载 (beforeunload) 调. */
export function cleanupBootListeners(): void {
	while (_unlisteners.length > 0) {
		const fn = _unlisteners.pop()!;
		try {
			fn();
		} catch (e) {
			// 单个 unlisten 失败不该阻断其它, 留痕
			console.warn('[boot] unlisten failed:', e);
		}
	}
}

/** 跑完整启动流程. 失败记录到 state.error 但不中断. */
export async function bootOnce(state: BootState): Promise<void> {
	const t0 = performance.now();
	for (const phase of _phases) {
		state.step = phase.step;
		state.progress = phase.progress;
		const t1 = performance.now();
		try {
			await phase.run();
			console.log(`[boot] ${phase.label} +${Math.round(performance.now() - t1)}ms`);
		} catch (e) {
			const traceId = reportError(e, { source: `boot.${phase.label}` });
			state.error = (state.error ? state.error + '\n' : '') + `${phase.label} 失败 (${traceId})`;
		}
	}
	// 切到第一个 session (有的话)
	const first = sessionStore.all[0];
	if (first) uiStore.switchToSession(first.id);
	state.step = 'ready';
	state.progress = 100;
	console.log(`[boot] done +${Math.round(performance.now() - t0)}ms`);
}
