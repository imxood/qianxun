// qianxun-desktop/src/lib/mock/projects.ts
// 3 个项目 mock

import type { Project } from '$lib/types/entity';

const NOW = '2026-06-07T22:00:00Z';
const HOUR_AGO = '2026-06-07T21:00:00Z';

export const mockProjects: Project[] = [
	{
		id: 'proj_qianxun_desktop',
		name: '千寻桌面端',
		folder: 'E:/git/maxu/qianxun/qianxun-desktop',
		provider: 'deepseek',
		default_model: 'deepseek-v4-flash',
		description: 'Tauri 2.0 + Svelte 5 桌面端',
		owner_id: 'u_1',
		created_at: '2026-05-30T08:00:00Z',
		last_active_at: NOW,
	},
	{
		id: 'proj_qianxun_daemon',
		name: '千寻 daemon',
		folder: 'E:/git/maxu/qianxun',
		provider: 'deepseek',
		default_model: 'deepseek-v4-flash',
		description: 'Rust daemon 进程 + HTTP API',
		owner_id: 'u_1',
		created_at: '2026-05-30T08:30:00Z',
		last_active_at: HOUR_AGO,
	},
	{
		id: 'proj_qianxun_test',
		name: 'qianxun-test',
		folder: 'E:/git/maxu/qianxun/qianxun-test',
		provider: 'deepseek',
		default_model: 'deepseek-v4-flash',
		description: '测试项目',
		owner_id: 'u_1',
		created_at: '2026-05-31T10:00:00Z',
		last_active_at: '2026-06-05T14:00:00Z',
	},
	{
		id: 'proj_scratch',
		name: 'scratch',
		folder: null,
		provider: 'deepseek',
		default_model: 'deepseek-v4-flash',
		owner_id: 'u_1',
		created_at: '2026-06-01T12:00:00Z',
		last_active_at: '2026-06-06T09:00:00Z',
	},
];
