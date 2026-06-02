// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — SessionStore (Stage 3 + Stage 4 offlineQueue)
// 与 docs/30_子项目规划/03-tauri-desktop.md §4.1.1 / §3.3 数据流图 / §10.3 离线队列 一致
// 与 docs/30_子项目规划/_shared-contract.md §3.2 SSE 事件 schema 一致
//
// Stage 3 范围:
//   - runtime: 单一 session (id / title / model / messages / isStreaming / abort / usage / stopReason / lastError)
//   - send(text, model): 推 user block → streamPrompt → handleEvent 处理 12 种事件
//   - cancel(): 调 AbortController.abort()
//   - handleEvent: 维护 indexMap (SSE event.index → messages[] array index) + 累积工具参数
//
// Stage 4 扩展 (不破坏 Stage 3 行为):
//   - offlineQueue: $state<OfflineMessage[]>, 持久化到 localStorage
//   - send(): 如 connectionStore.isDegraded → 入队而非发, toast.info
//   - flushOfflineQueue(): 重连后自动 flush (按 FIFO 顺序逐条 send)
//   - loadOfflineQueue() / persistOfflineQueue() / clearOfflineQueue() 内部 helper
//
// 渲染模型: messages 是一维 ContentBlock[]. ChatView 用 i%2 推断 role (Stage 3 简化).
// 未来 Stage 5 会改成 Message[] 模型 (role 在 Message 层级, 不在 block 层级).
// ───────────────────────────────────────────────────────────────────────────

import type { ContentBlock, SseEvent, StopReason } from "$lib/types/ipc";
import { SseError, streamPrompt } from "$lib/sse/client";
import { connectionStore } from "$lib/stores/connection.svelte";

/// 离线消息: 用户在 daemon 不可达时输入, 等重连后自动发送.
/// 与 docs/30_子项目规划/03-tauri-desktop.md §10.3 OfflineQueueItem 对齐
/// (id / sessionId / payload / createdAt / attempts), 简化 payload 直接用 text
/// (本阶段 message history 由 daemon 端按 sessionId 维护, 客户端只发增量).
export interface OfflineMessage {
	id: string; // crypto.randomUUID()
	sessionId: string;
	text: string;
	createdAt: string; // ISO 8601
	attempts: number;
	lastError?: string;
}

const OFFLINE_QUEUE_KEY = "qianxun.offline-queue";

interface SessionRuntimeState {
	sessionId: string;
	title: string;
	model: string;
	/// 累积: user text block + assistant 多个 block (text / thinking / tool_use / tool_result)
	messages: ContentBlock[];
	isStreaming: boolean;
	currentAbort: AbortController | null;
	usage: { input: number; output: number } | null;
	stopReason: StopReason | null;
	lastError: string | null;
}

function uuid(): string {
	if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
		return crypto.randomUUID();
	}
	// Fallback (older browsers / SSR)
	return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
		const r = (Math.random() * 16) | 0;
		const v = c === "x" ? r : (r & 0x3) | 0x8;
		return v.toString(16);
	});
}

class SessionStore {
	/// 单一活跃会话. Stage 3 简化, Stage 4 多会话时改为 sessionsById: Record<id, runtime>.
	runtime = $state<SessionRuntimeState | null>(null);

	/// 离线消息队列 (Stage 4 §10.3). 用户在 daemon 不可达时输入的 text,
	/// 重连后由 flushOfflineQueue() 按 FIFO 顺序 send. 持久化到 localStorage.
	offlineQueue = $state<OfflineMessage[]>([]);

	// ─── 内部状态 (非响应式) ────────────────────────────────────────────────

	/// SSE event.index → messages[] 数组下标的映射 (SSE 协议允许 index 任意顺序)
	#indexMap = new Map<number, number>();
	/// tool_use 流式参数累积 (event.index → 累计的 arguments_json 字符串)
	#argsAcc = new Map<number, string>();
	/// flush 排他锁 — 避免 reconnecting → connected → reconnecting 抖动时并发 flush
	#flushing = false;

	// ─── 派生 getter ────────────────────────────────────────────────────────

	get messages(): ContentBlock[] {
		return this.runtime?.messages ?? [];
	}
	get isStreaming(): boolean {
		return this.runtime?.isStreaming ?? false;
	}
	get currentAbort(): AbortController | null {
		return this.runtime?.currentAbort ?? null;
	}
	get offlineQueueSize(): number {
		return this.offlineQueue.length;
	}

	// ─── 公开方法 ──────────────────────────────────────────────────────────

	/// 启动新会话 (或延续当前) + 发送 user message.
	/// Stage 4: 如 connectionStore.isDegraded, 推入 offlineQueue 而非真发.
	async send(text: string, model: string): Promise<void> {
		if (!text.trim()) return;
		// 同一时刻只允许一个流
		if (this.runtime?.isStreaming) return;

		// ── Stage 4 §10.3: 离线入队 ───────────────────────────────────────
		if (connectionStore.isDegraded) {
			const msg: OfflineMessage = {
				id: uuid(),
				sessionId: this.runtime?.sessionId ?? "",
				text,
				createdAt: new Date().toISOString(),
				attempts: 0,
			};
			this.offlineQueue.push(msg);
			this.persistOfflineQueue();
			console.info(
				`[SessionStore] 消息入队 (queueSize=${this.offlineQueue.length}, daemonState=${connectionStore.daemonState})`
			);
			return;
		}

		const sessionId = this.runtime?.sessionId ?? uuid();
		const title = this.runtime?.title ?? text.slice(0, 40);
		const userBlock: ContentBlock = { type: "text", text };

		// 初始化 / 复用 runtime
		this.runtime = {
			sessionId,
			title,
			model,
			messages: this.runtime?.messages ?? [],
			isStreaming: true,
			currentAbort: new AbortController(),
			usage: this.runtime?.usage ?? null,
			stopReason: null,
			lastError: null,
		};
		this.runtime.messages.push(userBlock);

		// 清理本轮临时状态
		this.#indexMap.clear();
		this.#argsAcc.clear();

		try {
			await streamPrompt({
				// Stage 3: 固定 localhost. Stage 4 改为 connectionStore.daemonUrl.
				daemonUrl: connectionStore.daemonUrl,
				sessionId: this.runtime.sessionId,
				messages: [userBlock],
				model,
				onEvent: (e) => this.handleEvent(e),
				signal: this.runtime.currentAbort!.signal,
			});
		} catch (e) {
			if (this.runtime) {
				if (e instanceof SseError) {
					this.runtime.lastError = `[${e.code}] ${e.message}`;
				} else {
					this.runtime.lastError = (e as Error).message || String(e);
				}
			}
		} finally {
			if (this.runtime) {
				this.runtime.isStreaming = false;
				this.runtime.currentAbort = null;
			}
		}
	}

	/// 取消当前 streaming (调 AbortController.abort, streamPrompt 收到后立即 reject)
	cancel(): void {
		this.runtime?.currentAbort?.abort();
	}

	/// 显式重置 (Stage 3 暂未在 UI 暴露, 留 API 给未来 "新建会话" 按钮)
	reset(): void {
		this.cancel();
		this.runtime = null;
		this.#indexMap.clear();
		this.#argsAcc.clear();
	}

	// ─── Stage 4 §10.3: 离线队列管理 ────────────────────────────────────────

	/// 重连成功后自动 flush. FIFO 顺序逐条 send, 失败的 message.attempts++ 留在队尾.
	/// 排他锁: 抖动期 (reconnecting → connected → reconnecting) 不会并发 flush.
	async flushOfflineQueue(): Promise<void> {
		if (this.#flushing) return;
		if (this.offlineQueue.length === 0) return;
		if (connectionStore.isDegraded) {
			// 仍然不可达, 不 flush (下一轮 ping 通后再试)
			return;
		}

		this.#flushing = true;
		try {
			const snapshot = [...this.offlineQueue];
			console.info(`[SessionStore] flushOfflineQueue: ${snapshot.length} 条待发`);

			// 边发边修剪: 成功则从 offlineQueue 删除, 失败则 attempts++ 留在原位
			for (const msg of snapshot) {
				if (connectionStore.isDegraded) {
					// 中途又掉了, 停下, 剩下的留到下次
					console.warn("[SessionStore] flush 中途 daemon 再次失联, 中止");
					break;
				}
				if (this.runtime?.isStreaming) {
					// 当前流还没结束 (用户主动发了新消息), 让出, 留到下一轮
					console.info("[SessionStore] 正在流式, 让出 flush, 留到下一轮");
					break;
				}

				// 临时从队列移除 (成功就不回填, 失败时回填到队尾)
				const idx = this.offlineQueue.findIndex((m) => m.id === msg.id);
				if (idx >= 0) this.offlineQueue.splice(idx, 1);

				try {
					await this.send(msg.text, this.runtime?.model ?? "MiniMax-M3");
					// 成功: 不回填
				} catch (e) {
					// 失败: 回填到队尾
					msg.attempts += 1;
					msg.lastError = (e as Error).message;
					this.offlineQueue.push(msg);
					console.warn(
						`[SessionStore] flush 单条失败 (attempts=${msg.attempts}):`,
						e
					);
				}
			}

			this.persistOfflineQueue();
		} finally {
			this.#flushing = false;
		}
	}

	/// 用户清空队列 (Settings 提供按钮)
	clearOfflineQueue(): void {
		this.offlineQueue = [];
		this.persistOfflineQueue();
	}

	// ─── Stage 4 §10.3: localStorage 持久化 ──────────────────────────────────

	private persistOfflineQueue(): void {
		if (typeof localStorage === "undefined") return;
		try {
			localStorage.setItem(OFFLINE_QUEUE_KEY, JSON.stringify(this.offlineQueue));
		} catch {
			// ignore (private mode / quota)
		}
	}

	/// 启动时回填 (组件 onMount 调一次)
	loadOfflineQueue(): void {
		if (typeof localStorage === "undefined") return;
		try {
			const raw = localStorage.getItem(OFFLINE_QUEUE_KEY);
			if (!raw) return;
			const parsed = JSON.parse(raw) as OfflineMessage[];
			if (Array.isArray(parsed)) {
				this.offlineQueue = parsed;
			}
		} catch {
			// ignore (corrupt JSON — 清掉以免反复失败)
			try {
				localStorage.removeItem(OFFLINE_QUEUE_KEY);
			} catch {
				// ignore
			}
		}
	}

	// ─── 内部: SSE 事件处理 ────────────────────────────────────────────────

	handleEvent(event: SseEvent): void {
		const r = this.runtime;
		if (!r) return;

		switch (event.type) {
			// 1. message_start: 初始化本轮 assistant message 状态
			case "message_start": {
				this.#indexMap.clear();
				this.#argsAcc.clear();
				r.stopReason = null;
				r.usage = null;
				r.lastError = null;
				break;
			}

			// 2. content_block_start: 创建新 block, 记下 index → array 映射
			case "content_block_start": {
				let block: ContentBlock;
				if (event.block_type === "text") {
					block = { type: "text", text: "" };
				} else if (event.block_type === "thinking") {
					block = { type: "thinking", text: "" };
				} else {
					block = { type: "tool_use", id: "", name: "", input: {} };
				}
				r.messages.push(block);
				this.#indexMap.set(event.index, r.messages.length - 1);
				break;
			}

			// 3. text_delta: 追加到对应 text block
			case "text_delta": {
				const arrIdx = this.#indexMap.get(event.index);
				const block = arrIdx !== undefined ? r.messages[arrIdx] : undefined;
				if (block?.type === "text") {
					block.text = (block.text ?? "") + event.text;
				}
				break;
			}

			// 4. thinking_delta: 累积到 thinking block
			case "thinking_delta": {
				const arrIdx = this.#indexMap.get(event.index);
				const block = arrIdx !== undefined ? r.messages[arrIdx] : undefined;
				if (block?.type === "thinking") {
					block.text = (block.text ?? "") + event.text;
				}
				break;
			}

			// 5. tool_use_delta: 累积 arguments_json, 尽力解析为 input
			case "tool_use_delta": {
				const arrIdx = this.#indexMap.get(event.index);
				const block = arrIdx !== undefined ? r.messages[arrIdx] : undefined;
				if (block?.type === "tool_use") {
					if (event.id) block.id = event.id;
					if (event.name) block.name = event.name;
					const acc =
						(this.#argsAcc.get(event.index) ?? "") + (event.arguments_json ?? "");
					this.#argsAcc.set(event.index, acc);
					try {
						block.input = JSON.parse(acc);
					} catch {
						// 还没拼完整, 继续累积
					}
				}
				break;
			}

			// 6. tool_use_complete: 终结 tool_use 块
			case "tool_use_complete": {
				const arrIdx = this.#indexMap.get(event.index);
				const block = arrIdx !== undefined ? r.messages[arrIdx] : undefined;
				if (block?.type === "tool_use") {
					block.id = event.id;
					block.name = event.name;
					block.input = event.arguments;
				}
				this.#argsAcc.delete(event.index);
				break;
			}

			// 7. tool_result: 独立 block (不属于 assistant message content, 是 tool 的输出)
			case "tool_result": {
				const content =
					typeof event.content === "string"
						? event.content
						: JSON.stringify(event.content);
				r.messages.push({
					type: "tool_result",
					tool_use_id: event.tool_use_id,
					content,
					is_error: event.is_error,
					elapsed_ms: event.elapsed_ms,
				});
				break;
			}

			// 8. content_block_stop: 清理累积器
			case "content_block_stop": {
				this.#argsAcc.delete(event.index);
				break;
			}

			// 9. usage: token 计数
			case "usage": {
				r.usage = {
					input: event.input_tokens,
					output: event.output_tokens,
				};
				break;
			}

			// 10. message_delta: stop_reason
			case "message_delta": {
				r.stopReason = event.stop_reason;
				break;
			}

			// 11. message_stop: 流结束
			case "message_stop": {
				// 不重置 currentAbort — 由 send() 的 finally 统一清, 避免和外层 race
				break;
			}

			// 12. error: 服务端推送的错误, 落到 lastError
			case "error": {
				r.lastError = `[${event.code}] ${event.message}`;
				break;
			}
		}
	}
}

export const sessionStore = new SessionStore();
