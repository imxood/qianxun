// Tools API client
// 跟 docs/30_子项目规划/_shared-contract.md §3.1.1 对应.

import { apiGet, apiPost } from './client';
import type { ToolDefinition, ToolInvokeResult } from '$lib/types/api';

export interface ToolsResponse {
	tools: ToolDefinition[];
}

export interface ToolInvokeRequest {
	arguments: Record<string, unknown>;
}

export async function listTools(): Promise<ToolDefinition[]> {
	const r = await apiGet<ToolsResponse>('/v1/tools');
	return r.tools ?? [];
}

export async function invokeTool(
	name: string,
	arguments_: Record<string, unknown>
): Promise<ToolInvokeResult> {
	return apiPost<ToolInvokeResult>(`/v1/tools/${encodeURIComponent(name)}/invoke`, {
		arguments: arguments_
	});
}
