// qianxun-desktop/src/lib/types/entity.ts
// 跟 docs/daemon-design.md v1.0 §2.2 一字不差
// 设计基线: chat-first-redesign.md v1 + ADR-0002

// ─── 1. Project ──────────────────────────────────────────────
export interface Project {
	id: string; // "proj_xxx"
	name: string;
	folder: string | null; // "E:/git/maxu/qianxun/qianxun-desktop" | null = "Chat" 分类
	provider: string; // "deepseek"
	default_model: string; // "deepseek-v4-flash"
	description?: string;
	team_id?: string;
	owner_id: string;
	created_at: string; // ISO
	last_active_at: string; // ISO
}

// ─── 2. Session ──────────────────────────────────────────────
export type SessionStatus = 'Active' | 'Idle' | 'Archived';

export interface Session {
	id: string; // "sess_20260607_220000_123456"
	project_id: string | null; // FK → projects | null = "Chat" 分类
	title: string;
	provider: string;
	model: string;
	status: SessionStatus;
	message_count: number;
	owner_id: string;
	created_at: string;
	last_active_at: string;
}

// ─── 3. Plan ─────────────────────────────────────────────────
export type PlanStatus = 'Pending' | 'Running' | 'Done' | 'Failed' | 'Aborted';

export interface PlanTaskSpec {
	id: string;
	title: string;
	prompt: string;
	assigned_to: string; // "coder" | "tester" | "researcher"
	verified_by: string | null;
	verify_prompt: string | null;
	depends_on: string[];
	timeout_ms: number;
	output?: {
		type: 'text' | 'json' | 'files';
		expected_fields?: string[];
	};
}

export interface PlanContract {
	name: string;
	description: string;
	tasks: PlanTaskSpec[];
	timeout_ms: number;
}

export interface PlanAttachment {
	name: string;
	kind: 'file' | 'mr' | 'report';
	ref: string; // file path / MR id / url
}

export interface PlanResult {
	summary: string;
	tasks_completed: number;
	tasks_total: number;
	deliverables: string[]; // bullet list
}

export interface Plan {
	id: string; // "plan_xxx"
	session_id: string;
	contract: PlanContract;
	status: PlanStatus;
	started_at: string | null;
	ended_at: string | null;
	result: PlanResult | null;
	attachments: PlanAttachment[];
}

// ─── 4. SubSession ───────────────────────────────────────────
export type SubSessionStatus = 'Active' | 'Done' | 'Failed' | 'Aborted' | 'ReadOnly';

export interface SubSession {
	id: string; // "sub_xxx"
	plan_id: string;
	plan_task_id: string; // 1 个 SubSession = 1 个 PlanTask
	parent_session_id: string;
	role: string; // 跟 PlanTaskSpec.assigned_to 一致
	status: SubSessionStatus;
	messages: Message[]; // 独立上下文
	output: unknown; // JSON
	started_at: string;
	ended_at: string | null;
}

// ─── 5. Message ──────────────────────────────────────────────
export type MessageRole = 'user' | 'assistant' | 'system';

// 'task' = 原始任务消息 (默认); 'followup' = sub_session 完成后追加的追问, 不触发执行
export type MessageKind = 'task' | 'followup';

export interface ToolCall {
	id: string;
	name: string;
	arguments: Record<string, unknown>;
	result?: {
		content: string;
		is_error: boolean;
		elapsed_ms: number;
	};
	plan_ref?: string;
}

export interface Message {
	id: string;
	session_id: string | null; // 主会话消息
	sub_session_id: string | null; // 子会话消息
	role: MessageRole;
	content: string;
	tool_calls?: ToolCall[];
	plan_ref?: string; // assistant 消息引用的 plan
	kind?: MessageKind; // 默认 'task', 追问消息标记 'followup'
	created_at: string;
	streaming?: boolean; // mock 阶段: 是否正在流式输出
}

// ─── 6. ProjectExperience (走 qianxun-memory, mock 阶段 localStorage) ──
export interface ProjectExperience {
	id: string;
	project_id: string;
	content: string;
	source_session_id?: string;
	source_plan_id?: string;
	tags: string[];
	created_at: string;
}

// ─── 7. SessionMinute ────────────────────────────────────────
export interface SessionMinute {
	id: string;
	session_id: string;
	content: string; // 50-100 字摘要
	message_count_at_minute: number;
	created_at: string;
}

// ─── 8. 支撑: ChangedFile (Plan 完成时显示) ─────────────────
export type ChangeKind = '+' | '~' | '-';

export interface ChangedFile {
	kind: ChangeKind;
	path: string;
	task_id?: string; // 归属哪个 task
}

// ─── 9. 支撑: ScheduledTask (Col 1 定时任务段) ─────────────
export interface ScheduledTask {
	id: string;
	name: string;
	kind: 'memory_maintenance' | 'index_rebuild' | 'log_rotate';
	cron: string; // "0 3 * * *"
	enabled: boolean;
	last_run_at?: string;
	next_run_at?: string;
}
