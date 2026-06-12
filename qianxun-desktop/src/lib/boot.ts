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

import { chatStore } from './stores/chat.svelte';
import { planStore } from './stores/plan.svelte';
import { sessionStore } from './stores/session.svelte';
import { projectStore } from './stores/project.svelte';
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
			await Promise.all([chatStore.init(), planStore.init()]);
		},
	},
	{
		step: 'connecting',
		progress: 50,
		label: 'health',
		run: async () => {
			// 占位: 未来 health_check ping 走这里 (Phase B.1 修完后可调)
		},
	},
	{
		step: 'loading',
		progress: 80,
		label: 'lists',
		run: async () => {
			await Promise.all([sessionStore.init(), projectStore.loadAll()]);
		},
	},
];

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
