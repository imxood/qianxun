// Skills API client
// 跟 docs/30_子项目规划/_shared-contract.md §3.1.1 对应.

import { apiGet, apiPost } from './client';
import type { SkillSummary, SkillsReloadResult, SkillToggleResult } from '$lib/types/api';

export interface SkillsResponse {
	skills: SkillSummary[];
}

export async function listSkills(): Promise<SkillSummary[]> {
	const r = await apiGet<SkillsResponse>('/v1/skills');
	return r.skills ?? [];
}

export async function reloadSkills(): Promise<SkillsReloadResult> {
	return apiPost<SkillsReloadResult>('/v1/skills');
}

export async function toggleSkill(name: string): Promise<SkillToggleResult> {
	return apiPost<SkillToggleResult>(
		`/v1/skills/${encodeURIComponent(name)}/toggle`
	);
}
