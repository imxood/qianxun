// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — POST SSE 客户端
// 与 docs/30_子项目规划/03-tauri-desktop.md §8.1 / §8.2 / §8.3 完全一致
// 与 docs/30_子项目规划/_shared-contract.md §3.2 事件 schema 一致
//
// EventSource (浏览器原生) 只支持 GET + 无 body + 不可控重连. 我们要 POST +
// 请求体 + 可控重连, 所以用 fetch + ReadableStream 自实现.
//
// Stage 3 范围:
//   - streamPrompt() 单次发起 + 内置 3-6-12-30s 退避重试 (网络错误 / 5xx)
//   - 取消时不重试, 立即 reject SseError('cancelled')
//   - AbortSignal 触发立即停止
//   - 12 个 SseEvent 类型由 onEvent 回调逐个推送
//   - 不接 Tauri invoke (Stage 2 bridge 已建, 但本次直接 fetch,
//     因为 webview 自带 fetch, 减少 IPC 中转一次 — 与 03-tauri-desktop.md §3.2
//     "P0 走 Rust 中转" 决策的简化: 暂时直连, Stage 4 评估后再统一)
// ───────────────────────────────────────────────────────────────────────────

import type { ContentBlock, SseEvent } from "$lib/types/ipc";
import { parseSseStream } from "$lib/sse/parser";

export interface SsePromptOptions {
	daemonUrl: string;
	sessionId: string;
	/// 发送给 Daemon 的消息列表 (Stage 3 简化为只发当前 user message,
	/// 完整历史由 Daemon 端按 sessionId 自管).
	messages: ContentBlock[];
	model?: string;
	maxTokens?: number;
	temperature?: number;
	onEvent: (event: SseEvent) => void;
	/// 外部取消信号. 一旦 abort, 立即取消 fetch reader + 跳出重试.
	signal?: AbortSignal;
}

export type SseErrorCode = "network" | "parse" | "api" | "cancelled";

export class SseError extends Error {
	constructor(
		public code: SseErrorCode,
		message: string
	) {
		super(message);
		this.name = "SseError";
	}
}

// 退避: 3s → 6s → 12s → 30s (cap) + 0~20% 抖动 (03-tauri-desktop.md §8.3)
const BACKOFF_BASE_MS = [3_000, 6_000, 12_000, 30_000] as const;
const MAX_ATTEMPTS = 6; // ≥6 次后放弃, 上层处理

function backoffDelayMs(attempt: number): number {
	const idx = Math.min(Math.max(attempt - 1, 0), BACKOFF_BASE_MS.length - 1);
	const base = BACKOFF_BASE_MS[idx];
	const jitter = base * 0.2 * Math.random();
	return base + jitter;
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
	return new Promise((resolve) => {
		if (signal?.aborted) {
			resolve();
			return;
		}
		const t = setTimeout(() => {
			signal?.removeEventListener("abort", onAbort);
			resolve();
		}, ms);
		const onAbort = () => {
			clearTimeout(t);
			resolve();
		};
		signal?.addEventListener("abort", onAbort, { once: true });
	});
}

/// POST /v1/chat/session/:id/prompt, 流式读 SSE 事件.
/// - 3s → 6s → 12s → 30s 退避重试 (仅对网络错误 / 5xx 生效, 取消时不重试)
/// - AbortSignal 触发立即停止 fetch reader 并 reject SseError('cancelled')
/// - onEvent 回调: 每解析到一个完整 SSE 事件触发一次
/// - 流正常结束 (read done) → resolve (no error)
/// - 解析失败 → 警告 + 继续, 不中断流
export async function streamPrompt(opts: SsePromptOptions): Promise<void> {
	const url = `${opts.daemonUrl.replace(/\/$/, "")}/v1/chat/session/${encodeURIComponent(opts.sessionId)}/prompt`;
	const body = JSON.stringify({
		messages: opts.messages,
		model: opts.model,
		max_tokens: opts.maxTokens,
		temperature: opts.temperature,
	});

	let attempt = 0;

	while (true) {
		// 入口检查: 用户已经取消了?
		if (opts.signal?.aborted) {
			throw new SseError("cancelled", "Request cancelled by user");
		}

		attempt += 1;

		try {
			const response = await fetch(url, {
				method: "POST",
				headers: {
					"Content-Type": "application/json",
					Accept: "text/event-stream",
				},
				body,
				signal: opts.signal,
			});

			if (!response.ok) {
				// 5xx → 可重试; 4xx → 直接报错
				if (response.status >= 500 && attempt < MAX_ATTEMPTS) {
					await sleep(backoffDelayMs(attempt), opts.signal);
					continue;
				}
				throw new SseError("api", `HTTP ${response.status} from daemon`);
			}

			// 委托给 parser.ts 的 AsyncGenerator, 每个事件触发 onEvent 回调
			for await (const event of parseSseStream(response.body)) {
				opts.onEvent(event);
			}
			return; // 流正常结束
		} catch (e) {
			if (e instanceof SseError) {
				throw e;
			}
			if ((e as Error).name === "AbortError") {
				throw new SseError("cancelled", "Request cancelled");
			}
			// 网络错误 → 重试 (未超上限)
			if (attempt < MAX_ATTEMPTS) {
				await sleep(backoffDelayMs(attempt), opts.signal);
				continue;
			}
			throw new SseError("network", (e as Error).message || "Network error");
		}
	}
}

