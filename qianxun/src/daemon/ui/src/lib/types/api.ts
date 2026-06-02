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
