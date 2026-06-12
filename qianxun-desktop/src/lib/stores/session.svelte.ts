// qianxun-desktop/src/lib/stores/session.svelte.ts
// Session store (主会话)
//
// Stage 4a (sub-task #4): 切真后端 invoke.
//   - 初始化通过 init() async 方法调 listSessions() 拉真实 session 列表
//   - 切 session 时调 loadSession() 拉完整 conversation snapshot
//   - 删 buildSeed().sessions / buildSeed().messages 依赖 (mock 阶段)
//   - create() 仍客户端建, refresh() 拉新 (后端暂没 create_session RuntimeApi 方法)
//
// 业务约束:
//   - SessionInfo (后端 summary) 没 project_id / title / provider / owner_id 字段
//     → sessionInfoToSession() 用兜底值填充, UI 后续接 project 跟 owner
//   - 切 session 时, messages[id] 走 loadSession() 拿 conversation snapshot
//     → parse 留 sub-task #5 (需要后端 Conversation → Svelte Message 转换层)
//
// 关联:
//   - $lib/ipc/runtime.ts (listSessions / loadSession invoke)
//   - qianxun-runtime/src/api/types.rs (SessionInfo / SessionState DTO)
//   - docs/30_子项目规划/04b-tauri-runtime-integration.md §"数据流"

import {
	listSessions,
	loadSession,
	createSession,
	deleteSession,
	pauseSession,
	resumeSession,
	type SessionInfo,
} from '$lib/ipc/runtime';
import { uiStore } from './ui.svelte';
import { reportError } from '$lib/errors';
import { chatStore } from './chat.svelte';
import type { Session, Message } from '$lib/types/entity';

/// SessionInfo (后端 summary) → Session (前端 entity) 转换.
/// 后端没 title / provider / owner_id, 用兜底值.
/// 2026-06-09 加 project_root 透传 (从 SessionInfo.project_root → Session.project_id).
function sessionInfoToSession(info: SessionInfo): Session {
	return {
		id: info.id,
		project_id: info.project_root ?? null,
		title: info.id.length > 20 ? info.id.slice(0, 20) + '…' : info.id,
		provider: 'deepseek',
		model: info.model,
		// 后端 lowercase (active/paused/stored) → 前端 PascalCase (Active/Idle/Archived)
		status: info.status === 'active' ? 'Active' : info.status === 'paused' ? 'Idle' : 'Archived',
		message_count: info.message_count,
		owner_id: 'u_1',
		created_at: info.created_at,
		last_active_at: info.last_active_at,
	};
}

function createSessionStore() {
	const sessions = $state<Session[]>([]);
	const messages = $state<Record<string, Message[]>>({});

	let initialized = $state(false);
	let loading = $state(false);
	let lastError = $state<string | null>(null);
	// 2026-06-12 (批次 1.2): loadFullSession 进行中标记, UI 派生用
	// 防止切 session 时 history 加载完成前 ChatView 闪空窗
	const loadingFull = $state<Set<string>>(new Set());

	const activeSession = $derived.by(() => {
		const view = uiStore.activeView;
		return view.kind === 'session'
			? sessions.find((s) => s.id === view.session_id) ?? null
			: null;
	});

	const activeMessages = $derived(activeSession ? messages[activeSession.id] ?? [] : []);

	/// 启动时从后端拉 session 列表. 重复调用安全 (内部去重).
	/// 失败时保留空数组, 错误存 lastError, UI 弹 toast.
	/// 调用方: +page.svelte / +layout.svelte 的 onMount.
	async function init() {
		if (initialized || loading) return;
		loading = true;
		lastError = null;
		try {
			const r = await listSessions('all');
			sessions.length = 0;
			sessions.push(...r.sessions.map(sessionInfoToSession));
			initialized = true;
		} catch (e) {
			// lastError 存人类可读消息 (test/UI 状态), trace_id 在 toast 里 (用户报告用)
			lastError = e instanceof Error ? e.message : String(e);
			reportError(e, { source: 'sessionStore.init', toast: '加载会话失败' });
		} finally {
			loading = false;
		}
	}

	/// 强制刷新列表 (绕过 initialized 检查). 切完 session / 创建后调用.
	async function refresh() {
		initialized = false;
		await init();
	}

	/// 切 session 时调: 拉完整 conversation snapshot, 更新 message_count + 解析历史消息.
	///
	/// 2026-06-12 (Phase B.3): 解析后端 conversation_json (JSONL 格式) 写入 messages[id].
	/// JSONL 格式 (见 qianxun-core/src/agent/conversation.rs::to_jsonl_string):
	///   - 第 1 行: {"type":"system","prompt":"..."}
	///   - 后续每行: {"User":{...}} 或 {"Assistant":{...}} (serde external tag)
	///   - ContentBlock 数组简化: 取 text 字段拼接
	///
	/// 2026-06-12 (批次 1.2): 错误路径合并 — 单一入口 reportError (含 toast),
	/// 不再独立设 lastError (语义分离: lastError 留 sessionStore 级错误);
	/// 不再 throw (调用方 switchTo 也不用 .catch(() => {}) 吞); loading state
	/// 让 UI 显示骨架屏防空窗闪烁; finally 必清 loading, 完整执行流程.
	///
	/// 2026-06-12 (方案 C1 缓存): messages[id] !== undefined 时直接跳过 IPC.
	/// 切回已加载过的 session 不重 load, 收到新消息已走 chatStore.send → appendMessage
	/// 同步本地, 缓存自然新鲜. 失败路径不设 messages[id], 保留重试入口.
	async function loadFullSession(id: string): Promise<void> {
		// 缓存命中: 已加载过 (包括空数组, 后端没 turn 也会赋 []), 直接返回.
		if (messages[id] !== undefined) return;
		loadingFull.add(id);
		try {
			const state = await loadSession(id);
			const local = sessions.find((s) => s.id === id);
			if (local) {
				local.message_count = state.message_count;
			}
			if (state.conversation_json) {
				// 幂等: 二次加载不覆盖已有消息的 created_at (批次 1.4: 后端 Message 无 created_at,
				// 解析时统一用 now 兜底, 不幂等会让历史消息时间戳每次刷新都变).
				const fresh = parseConversationJsonl(state.conversation_json, id);
				const existing = messages[id] ?? [];
				const existingById = new Map(existing.map((m) => [m.id, m]));
				messages[id] = fresh.map((m) => {
					const prev = existingById.get(m.id);
					return prev ? { ...m, created_at: prev.created_at } : m;
				});
			} else {
				// 后端没 snapshot (刚 create 没 turn / load 返空字符串) → 显式赋 [] 标记已加载,
				// 避免切回 session 重复 IPC 拿空字符串. 空字符串前端无法判断 "已查过没数据" vs "还没查".
				messages[id] = [];
			}
		} catch (e) {
			// 单一入口: reportError 上报 (含 toast), 不再独立设 lastError
			reportError(e, {
				source: 'sessionStore.loadFullSession',
				toast: '加载会话详情失败',
				context: { session_id: id },
			});
		} finally {
			loadingFull.delete(id);
		}
	}

	return {
		get all() {
			return sessions;
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
		/// 2026-06-12 (批次 1.2): loadFullSession 进行中标记, UI 派生用.
		/// 用法: `{#if sessionStore.loadingFull.has(active.id)} ... 加载骨架屏 ...`
		get loadingFull(): ReadonlySet<string> {
			return loadingFull;
		},
		get(id: string): Session | undefined {
			return sessions.find((s) => s.id === id);
		},
		byProject(projectId: string | null): Session[] {
			if (projectId === null) return sessions.filter((s) => s.project_id === null);
			return sessions.filter((s) => s.project_id === projectId);
		},
		get active(): Session | null {
			return activeSession;
		},
		get activeMessages(): Message[] {
			return activeMessages;
		},
		create(opts: {
			project_id: string | null;
			folder?: string | null;
			title?: string;
			provider?: string;
			model?: string;
		}): Promise<Session> {
			// 调 invoke 'create_session' 让后端生成 sess_ 格式 ID, 避免客户端/后端 ID 命名空间脱节
			// (旧实现: 客户端造 ID, send_message 必 404 "session not found").
			// 后端: RuntimeApi::create_session → agent_host.create_session + store.create.
			//
			// 错误处理 (2026-06-09): invoke 失败 (网络/鉴权/服务端 bug 等) 时,
			// 错误存 lastError, UI 弹 toast 提示. 不向上抛, 避免 uncaught promise rejection.
			return (async () => {
				let info: Awaited<ReturnType<typeof createSession>>;
				try {
					info = await createSession({
						model: opts.model,
						project_root: opts.project_id ?? undefined,
					});
				} catch (e) {
					lastError = e instanceof Error ? e.message : String(e);
					reportError(e, { source: 'sessionStore.create', toast: '新建会话失败' });
					throw e; // 让调用方 (NewTaskButton) 仍能 try/catch 处理 UI 状态
				}
				const newSession = sessionInfoToSession(info);
				sessions.push(newSession);
				messages[newSession.id] = [];
				return newSession;
			})();
		},
		switchTo(id: string) {
			const s = sessions.find((x) => x.id === id);
			if (s) {
				s.last_active_at = new Date().toISOString();
				uiStore.switchToSession(id);
				// 2026-06-12 (批次 2.4): loadFullSession 内部已 reportError + finally 清 loading,
				// 这里不再需要 .catch(() => {}) 吞错. UI 通过 loadingFull 派生显示骨架屏.
				void loadFullSession(id);
			}
		},

		/// 删除 session (调 invoke + 本地从 store 移除 + 清 messages).
		/// 2026-06-12 (批次 2.3): 联动 chatStore.forgetUserMessage 释放 resend 缓存, 避免 Map 累积泄漏.
		async delete(id: string): Promise<void> {
			await deleteSession(id);
			const idx = sessions.findIndex((s) => s.id === id);
			if (idx >= 0) sessions.splice(idx, 1);
			delete messages[id];
			chatStore.forgetUserMessage(id);
			// 如果删的是当前 active session, 切到空状态
			const view = uiStore.activeView;
			if (view.kind === 'session' && view.session_id === id) {
				uiStore.switchToNew(null);
			}
		},

		/// 暂停 session. 后续 send_message 会被后端拒 (InvalidRequest).
		async pause(id: string): Promise<void> {
			await pauseSession(id);
			const s = sessions.find((x) => x.id === id);
			if (s) s.status = 'Idle';
		},

		/// 解除暂停.
		async resume(id: string): Promise<void> {
			await resumeSession(id);
			const s = sessions.find((x) => x.id === id);
			if (s) s.status = 'Active';
		},
		getMessages(sessionId: string): Message[] {
			return messages[sessionId] ?? [];
		},
		appendMessage(sessionId: string, msg: Message) {
			if (!messages[sessionId]) messages[sessionId] = [];
			messages[sessionId].push(msg);
			const s = sessions.find((x) => x.id === sessionId);
			if (s) {
				s.message_count = messages[sessionId].length;
				s.last_active_at = msg.created_at;
			}
		},
		countByProject(projectId: string): number {
			return sessions.filter((s) => s.project_id === projectId).length;
		},
		init,
		refresh,
		loadFullSession,
		/// 测试专用: 重置内部状态 (call in beforeEach).
		/// 业务代码不应该调. 包装内部 $state 重新赋值.
		__resetForTesting() {
			sessions.length = 0;
			for (const k of Object.keys(messages)) delete messages[k];
			initialized = false;
			loading = false;
			lastError = null;
		},
	};
}

export const sessionStore = createSessionStore();

/// JSONL conversation 解析 (2026-06-12 加, Phase B.3; 批次 1.1 容错升级).
///
/// 后端 conversation_json 是 JSONL 字符串, 每行一条记录. 详见 qianxun-core
/// src/agent/conversation.rs::to_jsonl_string.
///
/// 契约 (2026-06-12 批次 1.1, 对齐 Rust from_jsonl_str):
///   - 损坏行静默 skip (不阻断整体加载, Rust 端 if let Ok 行为)
///   - system 行 (header) 跳过, 不解析 content (TS 暂不渲染 system_prompt)
///   - 非 User/Assistant tag 行静默 skip
///   - 空格容错: serde 序列化器输出 `{"type": "system"` (key 后有空格),
///     老 startsWith('{"type":"system"') 不带空格匹配失败; 改宽松.
function parseConversationJsonl(jsonl: string, sessionId: string): Message[] {
	const result: Message[] = [];
	const lines = jsonl.split('\n');
	for (const line of lines) {
		const trimmed = line.trim();
		if (!trimmed) continue;
		// system header 行: 检测 "type" 字段 (不管 key 跟 value 间有没有空格)
		if (trimmed.startsWith('{"type"') && trimmed.includes('"system"')) continue;
		// 损坏行 try/catch 静默 skip, 对齐 Rust 行为
		let parsed: Record<string, { id: string; content: ContentBlock[] }>;
		try {
			parsed = JSON.parse(trimmed) as typeof parsed;
		} catch {
			continue;
		}
		// serde external tag: {"User":{...}} / {"Assistant":{...}}
		const tag = Object.keys(parsed)[0];
		if (tag !== 'User' && tag !== 'Assistant') continue;
		const inner = parsed[tag]!;
		result.push({
			id: inner.id,
			role: tag === 'User' ? 'user' : 'assistant',
			content: extractTextFromContentBlocks(inner.content),
			// 2026-06-12 (批次 1.4): 后端 qianxun-core/src/agent/message.rs Message struct
			// 没有 created_at 字段, 序列化不携带; TS 端统一用 now 兜底.
			// 二次 loadFullSession 由 messages[id] 幂等逻辑保护, 不覆盖已存在的 created_at.
			created_at: new Date().toISOString(),
			session_id: sessionId,
			sub_session_id: null, // 主会话消息, 跟 Message 类型契约一致
		});
	}
	return result;
}

/// ContentBlock 简化提取: 只取 text 字段拼接. 其它类型 (tool_use / tool_result)
/// 暂不在 UI 渲染, 完整解析留 v0.4.
function extractTextFromContentBlocks(blocks: ContentBlock[]): string {
	return blocks
		.filter((b): b is { type: 'text'; text: string } => b.type === 'text')
		.map((b) => b.text)
		.join('\n');
}

/// ContentBlock Rust serde 反序列化形态. 跟 qianxun-core/src/agent/message.rs 1:1.
type ContentBlock =
	| { type: 'text'; text: string }
	| { type: 'tool_use'; id: string; name: string; input: unknown }
	| { type: 'tool_result'; tool_use_id: string; content: string; is_error: boolean };

// 2026-06-12 (批次 1.5): 仅测试用 export, 业务代码不直接 import. 不污染运行时 API 表面.
export { parseConversationJsonl };
