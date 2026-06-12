// qianxun-desktop/src/lib/errors.ts
// 统一错误上报 (2026-06-12 桌面端 UI 完整性修复).
//
// 单一入口: reportError(e, { source, toast?, context? }).
// 行为:
//   1. console.error 留痕, 带 trace_id + source + context, 排查用
//   2. (可选) 弹 toast, 标题 + trace_id 短码, 用户可贴日志
//   3. 返回 trace_id, 调用方可在调试时也存一份
//
// 用法:
//   catch (e) { reportError(e, { source: 'sessionStore.init', toast: '加载会话失败' }); }
//   catch (e) { reportError(e, { source: 'persist.read', context: { key } }); }  // 静默 (无 toast)
//
// 设计原则 (用户约束 #2 错误处理要准确易读且详细):
//   - trace_id 短码 (e_xxxxx_yyyy) 关联 console 和 toast, 用户贴日志就能反查
//   - toast 仅在用户能感知失败时弹, 静默失败 (parse/refresh) 不弹避免误报淹没
//   - 单文件单函数, 不拆 helper

import { uiStore } from './stores/ui.svelte';

let _seq = 0;
const _newTraceId = (): string => `e_${Date.now().toString(36)}_${(++_seq).toString(36)}`;

export interface ReportErrorOpts {
	/** 调用源标签, 形如 'sessionStore.init' / 'planStore.cancel' / 'boot.lists'. */
	source: string;
	/** 弹 toast 标题, 省略则不弹 (静默失败场景). */
	toast?: string;
	/** 附加上下文 (e.g. { sessionId, planId }), 自动写 console. */
	context?: Record<string, unknown>;
}

/** 上报错误. 返回 trace_id 短码 (e_xxxxx_yyyy 格式). */
export function reportError(e: unknown, opts: ReportErrorOpts): string {
	const traceId = _newTraceId();
	const err = e instanceof Error ? e : new Error(String(e));
	console.error(
		`[${traceId}] ${opts.source} failed:`,
		err.message,
		opts.context ?? '',
	);
	if (opts.toast) {
		uiStore.pushToast({
			kind: 'error',
			title: opts.toast,
			description: `${err.message} (${traceId})`,
			timeout_ms: 5000,
		});
	}
	return traceId;
}
