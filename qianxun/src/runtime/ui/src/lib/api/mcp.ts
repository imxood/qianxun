// MCP API client
// 跟 docs/30_子项目规划/_shared-contract.md §3.1.1 对应.

import { apiDelete, apiGet, apiPost } from './client';
import type { McpServerConfig, McpServerSummary, McpTestResult } from '$lib/types/api';

export interface McpServersResponse {
	servers: McpServerSummary[];
}

export interface McpServerResponse {
	server: McpServerConfig;
}

export interface McpStatusResponse {
	status: 'added' | 'deleted';
}

export async function listMcpServers(): Promise<McpServerSummary[]> {
	const r = await apiGet<McpServersResponse>('/v1/mcp/servers');
	return r.servers ?? [];
}

export async function addMcpServer(cfg: McpServerConfig): Promise<void> {
	await apiPost<McpStatusResponse>('/v1/mcp/servers', cfg);
}

export async function deleteMcpServer(id: string): Promise<void> {
	await apiDelete<McpStatusResponse>(`/v1/mcp/servers/${encodeURIComponent(id)}`);
}

export async function testMcpServer(id: string): Promise<McpTestResult> {
	return apiPost<McpTestResult>(`/v1/mcp/servers/${encodeURIComponent(id)}/test`);
}
