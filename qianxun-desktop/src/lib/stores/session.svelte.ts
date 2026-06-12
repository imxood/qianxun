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
	async function loadFullSession(id: string) {
		try {
			const state = await loadSession(id);
			// 找到本地 session, 更新 message_count
			const local = sessions.find((s) => s.id === id);
			if (local) {
				local.message_count = state.message_count;
			}
			// 解析 conversation_json → Message[] 写入 messages[id]
			if (state.conversation_json) {
				messages[id] = parseConversationJsonl(state.conversation_json, id);
			}
			return state;
		} catch (e) {
			lastError = e instanceof Error ? e.message : String(e);
			reportError(e, {
				source: 'sessionStore.loadFullSession',
				toast: '加载会话详情失败',
				context: { session_id: id },
			});
			throw e;
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
				// 切 session 时拉完整状态 (失败 swallow, 不阻塞 UI)
				void loadFullSession(id).catch(() => {});
			}
		},

		/// 删除 session (调 invoke + 本地从 store 移除 + 清 messages).
		async delete(id: string): Promise<void> {
			await deleteSession(id);
			const idx = sessions.findIndex((s) => s.id === id);
			if (idx >= 0) sessions.splice(idx, 1);
			delete messages[id];
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

/// JSONL conversation 解析 (2026-06-12 加, Phase B.3).
///
/// 后端 conversation_json 是 JSONL 字符串, 每行一条记录. 详见 qianxun-core
/// src/agent/conversation.rs::to_jsonl_string. 解析失败抛 Error, 调用方捕.
function parseConversationJsonl(jsonl: string, sessionId: string): Message[] {
	const result: Message[] = [];
	const lines = jsonl.split('\n');
	for (const line of lines) {
		const trimmed = line.trim();
		if (!trimmed) continue;
		// 第 1 行: 系统提示头, 跳过
		if (trimmed.startsWith('{"type":"system"')) continue;
		const parsed = JSON.parse(trimmed) as Record<string, { id: string; content: ContentBlock[] }>;
		// serde external tag: {"User":{...}} / {"Assistant":{...}}
		const tag = Object.keys(parsed)[0];
		if (tag !== 'User' && tag !== 'Assistant') continue;
		const inner = parsed[tag]!;
		result.push({
			id: inner.id,
			role: tag === 'User' ? 'user' : 'assistant',
			content: extractTextFromContentBlocks(inner.content),
			created_at: new Date().toISOString(), // 后端未传 ts, 暂用现在
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
