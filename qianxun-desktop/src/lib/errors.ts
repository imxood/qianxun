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
//
// 2026-06-12 (批次 3.1) 加 toast throttle: 同 source 1s 内最多 THROTTLE_MAX 条 toast,
// 错误风暴时 (WS 断连 1s 1000 事件) 不刷屏. console 仍全量 (排查用).
// 实现: 简单 Map + window 滑动, 不引第三方库 (规范 4 反对低质量代码 → 不引 token bucket).

import { uiStore } from './stores/ui.svelte';

let _seq = 0;
const _newTraceId = (): string => `e_${Date.now().toString(36)}_${(++_seq).toString(36)}`;

/// 2026-06-12 (批次 3.1) throttle state: per-source 1s 滑动窗口, 累计被压条数.
const _throttle = new Map<string, { count: number; firstAt: number; suppressed: number }>();
const THROTTLE_WINDOW_MS = 1000;
const THROTTLE_MAX = 3;

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
	// 1. console 永远写 (不 throttle, 排查用, 规范 1 错误处理要详细)
	console.error(
		`[${traceId}] ${opts.source} failed:`,
		err.message,
		opts.context ?? '',
	);
	// 2. toast throttle: 同 source 1s 内最多 THROTTLE_MAX 条, 后续折叠成 "还有 N 条被压"
	if (opts.toast) {
		const now = Date.now();
		const key = opts.source;
		const prev = _throttle.get(key);
		if (prev && now - prev.firstAt < THROTTLE_WINDOW_MS) {
			prev.count++;
			if (prev.count === THROTTLE_MAX + 1) {
				// 正好越过阈值: 弹一条汇总 toast 告知用户"还在出错, 已被压"
				uiStore.pushToast({
					kind: 'error',
					title: opts.toast,
					description: `还有更多同源错误被压, 见 console (${traceId})`,
					timeout_ms: 5000,
				});
			} else if (prev.count <= THROTTLE_MAX) {
				// 阈值内: 弹正常 toast
				uiStore.pushToast({
					kind: 'error',
					title: opts.toast,
					description: `${err.message} (${traceId})`,
					timeout_ms: 5000,
				});
			}
			// 超出不弹, 但 console 还在
		} else {
			_throttle.set(key, { count: 1, firstAt: now, suppressed: 0 });
			uiStore.pushToast({
				kind: 'error',
				title: opts.toast,
				description: `${err.message} (${traceId})`,
				timeout_ms: 5000,
			});
		}
	}
	return traceId;
}
