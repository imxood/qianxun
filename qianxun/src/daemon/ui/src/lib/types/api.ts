// Stage 7a — 跟 daemon 共享的 schema (TypeScript 类型定义)
// 跟 docs/30_子项目规划/_shared-contract.md §3.1.1 对应
// 跟 docs/30_子项目规划/01b-daemon-web-console.md §4.2 对应
//
// 注意: Web Console 是纯前端, 这些类型由前端根据契约文档独立维护.
// daemon 端的 struct 字段是 source of truth, 这里只对应公共字段.

// ─── LLM Provider ─────────────────────────────────────────────────────

/** LLM provider 配置 (POST/PUT body, GET 单个) */
export interface LlmProviderConfig {
	id: string;
	provider: string; // 'deepseek' | 'anthropic' | 'minimax' | 'openai' | ...
	model: string;
	base_url?: string;
	api_key?: string; // POST/PUT 才带, GET 单个 detail 时不返
}

/** LLM provider 列表项 (GET 列表, **不**含 key) */
export interface LlmProviderSummary {
	id: string;
	provider: string;
	model: string;
	base_url?: string;
	has_key: boolean;
	active: boolean;
}

/** Provider 测试连接结果 */
export interface ProviderTestResult {
	ok: boolean;
	latency_ms?: number;
	error?: string;
	model_version?: string;
}

// ─── Skills ──────────────────────────────────────────────────────────

export type SkillEnabled = 'enabled' | 'disabled';

export interface SkillSummary {
	name: string;
	description: string;
	enabled: boolean;
	path: string;
	version?: string;
	frontmatter?: Record<string, unknown>;
}

/** Skills 重载结果 */
export interface SkillsReloadResult {
	status: 'reloaded';
	count: number;
}

/** Skills toggle 结果 */
export interface SkillToggleResult {
	status: SkillEnabled;
}

// ─── MCP ─────────────────────────────────────────────────────────────

export type McpTransport = 'stdio' | 'http';

export interface McpServerConfig {
	id: string;
	name: string;
	transport: McpTransport;
	/** stdio 模式: 命令; http 模式: URL */
	command_or_url: string;
	args?: string[];
	env?: Record<string, string>;
}

export interface McpServerSummary {
	id: string;
	name: string;
	transport: McpTransport;
	command_or_url: string;
	connected: boolean;
	tool_count: number;
}

/** MCP 测试连接结果 */
export interface McpTestResult {
	ok: boolean;
	tools?: { name: string; description?: string }[];
	error?: string;
}

// ─── Tools ───────────────────────────────────────────────────────────

export interface ToolDefinition {
	name: string;
	description: string;
	/** JSON Schema, 用于试用时的 input 校验 */
	input_schema: Record<string, unknown>;
}

/** Tool 试用 (invoke) 结果 */
export interface ToolInvokeResult {
	output: string;
	elapsed_ms?: number;
	error?: string;
}

// ─── System ─────────────────────────────────────────────────────────

export interface SystemStatus {
	status: string;
	version: string;
	stage: string;
}

export interface SystemHealth {
	status: 'ok' | 'degraded' | 'down';
}

// ─── Stage 7b ─── Memory / Sessions / Config / System Metrics ───────

// ── Memory ──

/** 单个 memory session 摘要 (在列表里展示) */
export interface MemorySessionSummary {
	id: string;
	created_at?: string;
	last_active?: string;
	observation_count: number;
	preview?: string;
}

/** 单条 observation (memory 引擎里的"观察"记录) */
export interface MemoryObservation {
	id: string;
	session_id: string;
	/// ISO 8601 字符串, 例如 "2026-06-03T00:00:00Z".
	/// 跟 daemon `qianxun-memory::Observation.timestamp` 1:1 对齐.
	timestamp: string;
	/// JSON 字符串 (frontend 自己 JSON.parse). 跟 daemon `Observation.data` 1:1.
	/// 留 TEXT 是为了 (a) 减少内存拷贝 (b) 跨版本 schema 演进.
	data: string;
	created_at: string;
}

/** 搜索请求 body */
export interface MemorySearchRequest {
	query: string;
	limit?: number;
	tags?: string[];
}

/** 搜索结果 */
export interface MemorySearchResult {
	id: string;
	session_id: string;
	content: string;
	tags?: string[];
	score?: number;
	created_at?: string;
}

/** 搜索响应 */
export interface MemorySearchResponse {
	results: MemorySearchResult[];
	total?: number;
}

/** session 列表响应 */
export interface MemorySessionsResponse {
	sessions: MemorySessionSummary[];
}

// ── Sessions (chat) ──

export type SessionStatus = 'active' | 'paused' | 'completed' | 'cancelled';

export interface ChatSessionSummary {
	id: string;
	model: string;
	created_at: string;
	last_active: string;
	message_count: number;
	status: SessionStatus;
	token_usage: {
		input: number;
		output: number;
		total: number;
	};
}

export interface ChatSessionsResponse {
	sessions: ChatSessionSummary[];
	total: number;
}

export interface ChatSessionEvent {
	ts: string;
	kind: string;
	role?: string;
	content?: string;
	tokens?: number;
}

export interface ChatSessionDetail {
	id: string;
	model: string;
	created_at: string;
	last_active: string;
	status: SessionStatus;
	messages: ChatSessionEvent[];
	token_usage: ChatSessionSummary['token_usage'];
}

export interface ChatSessionActionResponse {
	status: 'cancelled' | 'paused' | 'deleted' | 'ok';
	id: string;
}

// ── Config ──

/** ResolvedConfig 单个 provider 项 (跟 daemon 端 ResolvedProviderConfig 兼容) */
export interface ConfigProvider {
	id: string;
	provider: string;
	model: string;
	base_url?: string;
	has_key: boolean;
	active: boolean;
}

/** ResolvedConfig 视图 (Web UI 看的子集) */
export interface ResolvedConfigView {
	active_provider: string;
	log_level: string;
	max_sessions: number;
	providers: ConfigProvider[];
	skills_dirs: string[];
	memory_dir?: string;
	disabled_skills?: string[];
	[key: string]: unknown; // 额外字段保留
}

/** PUT /v1/config 响应 */
export interface ConfigUpdateResponse {
	status: 'updated';
	requires_reload: boolean;
	changed_fields: string[];
}

// ── System Metrics ──

/** GET /v1/system/metrics 响应 */
export interface SystemMetrics {
	cpu_percent: number;
	mem_mb: number;
	uptime_s: number;
	active_conns: number;
	sessions: {
		active: number;
		paused: number;
		total: number;
	};
	/** 最近 1 分钟每秒的 conns 数 (用于折线图) */
	conns_history?: number[];
	ts: string;
}

/** GET /v1/system/logs?lines=100 响应 */
export interface SystemLogsResponse {
	lines: string[];
	total: number;
}

// ── Stage 9c ── Settings (token rotate) ─────────────────────────────

/** POST /v1/system/admin/rotate-token 响应 */
export interface TokenRotateResponse {
	/** 新签发的 admin JWT (HS256, sub="admin", exp=now+24h) */
	token: string;
	/** 过期 unix 时间戳 (秒) */
	exp: number;
	/** 用户名 (sub) */
	sub: string;
	/** 有效时长 (秒) — 给前端展示用, 等于 exp - now */
	expires_in: number;
}
