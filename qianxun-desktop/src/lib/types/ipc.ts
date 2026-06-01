// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — IPC 类型定义
// 与 docs/30_子项目规划/03-tauri-desktop.md §7 完全一致
// 与 docs/30_子项目规划/_shared-contract.md §6 数据模型一致
//
// Stage 1 (当前):  仅定义 TypeScript 类型, 不接 Tauri 2.0
// Stage 2 (后续):  通过 @tauri-apps/api/core.invoke 消费
// ───────────────────────────────────────────────────────────────────────────

// ISO 8601 UTC 时间戳
export type ISODateString = string;

// ─── 连接 / 系统 ────────────────────────────────────────────────────────────

/**
 * Daemon 健康状态.
 * 与 03-tauri-desktop.md §4.1.2 ConnectionStore.daemonState / §10.1 UI 状态机完全统一.
 */
export type DaemonState = "offline" | "reconnecting" | "degraded" | "connected";

export interface HealthStatus {
	/// 顶层 daemon state (4 态)
	status: DaemonState;
	version: string;
	uptime_sec: number;
	session_count: number;
	mcp_online: number;
	provider_status: Record<string, "ok" | "rate_limited" | "down">;
}

export interface StatusInfo {
	status: "running" | "starting" | "stopping";
	version: string;
	build: string;
	features: string[];
}

// ─── 项目 (Project) ─────────────────────────────────────────────────────────

export interface Project {
	id: string; // "proj_xxx"
	name: string;
	path: string; // 工作目录
	description?: string;
	team_id?: string; // 关联到 team
	owner_id: string; // user_id
	created_at: ISODateString;
}

// ─── 会话 (Session) ──────────────────────────────────────────────────────────

export type SessionStatus = "active" | "idle" | "archived";

export interface Session {
	id: string; // "sess_xxx"
	project_id: string;
	title: string;
	model: string;
	status: SessionStatus;
	created_at: ISODateString;
	last_active_at: ISODateString;
	message_count: number;
	owner_id: string;
}

export interface SessionCreateOpts {
	title?: string;
	thinking_budget?: number;
	mode?: "auto" | "plan";
	system_prompt_override?: string;
}

// ─── 团队 (Team) ────────────────────────────────────────────────────────────

export type TeamRole = "owner" | "admin" | "developer" | "viewer";

export interface TeamMember {
	user_id: string;
	display_name: string;
	email?: string;
	avatar_url?: string;
	role: TeamRole;
	joined_at: ISODateString;
}

export interface Team {
	id: string; // "team_xxx"
	name: string;
	members: TeamMember[]; // 初始 inline, 后续规范化
	created_at: ISODateString;
}

export interface ProjectAssignment {
	team_id: string;
	project_id: string;
	member_ids: string[];
	assigned_at: ISODateString;
}

// ─── 消息 (Message) — Track C 扩展 ──────────────────────────────────────────

export type MessageRole = "user" | "assistant" | "system" | "tool";
export type ContentBlockType = "text" | "thinking" | "tool_use" | "tool_result" | "image";

export interface ContentBlock {
	type: ContentBlockType;
	// text / thinking
	text?: string;
	// tool_use
	id?: string; // "toolu_xxx"
	name?: string; // "read_file"
	input?: Record<string, unknown>;
	// tool_result
	tool_use_id?: string;
	content?: string | ContentBlock[];
	is_error?: boolean;
	elapsed_ms?: number;
	// image
	source?: { type: "base64" | "url"; media_type: string; data: string };
}

export interface TokenUsage {
	input: number;
	output: number;
	cache_creation_input?: number;
	cache_read_input?: number;
}

export type StopReason =
	| "end_turn"
	| "max_tokens"
	| "stop_sequence"
	| "tool_use"
	| "content_filtered"
	| "cancelled"
	| "error"
	| "unknown";

export interface Message {
	id: string; // "msg_xxx" (前端生成 uuid v4)
	session_id: string;
	role: MessageRole;
	content: ContentBlock[];
	model?: string;
	usage?: TokenUsage;
	stop_reason?: StopReason;
	created_at: ISODateString;
	/// 客户端本地状态 (不持久化到 Daemon)
	streaming?: boolean;
	error?: string;
}

export interface PromptBody {
	messages: ContentBlock[];
	model?: string;
	max_tokens?: number;
	temperature?: number;
	thinking?: { enabled: boolean; budget_tokens?: number };
	metadata?: Record<string, unknown>;
}

// ─── 工具 / Skills / MCP ─────────────────────────────────────────────────────

export interface Tool {
	name: string;
	description: string;
	input_schema: Record<string, unknown>;
}

export interface Skill {
	name: string;
	description: string;
	path: string;
	frontmatter: Record<string, unknown>;
}

export interface McpServer {
	name: string;
	command: string;
	args: string[];
	env: Record<string, string>;
	online: boolean;
}

// ─── IPC 错误统一 ────────────────────────────────────────────────────────────

export type IpcError =
	| { code: "NotFound"; message: string }
	| { code: "ServiceUnavailable"; message: string }
	| { code: "Unauthenticated"; message: string }
	| { code: "Forbidden"; message: string }
	| { code: "RateLimited"; retry_after_sec?: number; message: string }
	| { code: "ApiError"; status: number; message: string }
	| { code: "Internal"; message: string };
