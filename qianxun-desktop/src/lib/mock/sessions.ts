// qianxun-desktop/src/lib/mock/sessions.ts
// 6 个 session mock, 跨 3 项目

import type { Session } from '$lib/types/entity';

const NOW = '2026-06-07T22:00:00Z';
const HOUR_AGO = '2026-06-07T21:00:00Z';
const TODAY_MORNING = '2026-06-07T09:00:00Z';
const YESTERDAY = '2026-06-06T15:00:00Z';
const TWO_DAYS = '2026-06-05T11:00:00Z';
const LAST_WEEK = '2026-06-02T14:00:00Z';

export const mockSessions: Session[] = [
	// 千寻桌面端 - 当前活跃 (Plan 进行中)
	{
		id: 'sess_jwt_auth',
		project_id: 'proj_qianxun_desktop',
		title: '实现 JWT 用户认证',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		status: 'Active',
		message_count: 14,
		owner_id: 'u_1',
		created_at: TODAY_MORNING,
		last_active_at: NOW,
	},
	// 千寻桌面端 - 完成
	{
		id: 'sess_dark_mode',
		project_id: 'proj_qianxun_desktop',
		title: '加个 dark mode 顺便修对齐',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		status: 'Idle',
		message_count: 8,
		owner_id: 'u_1',
		created_at: YESTERDAY,
		last_active_at: YESTERDAY,
	},
	// 千寻桌面端 - 完成
	{
		id: 'sess_perf_opt',
		project_id: 'proj_qianxun_desktop',
		title: 'Svelte 5 渲染性能优化',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		status: 'Idle',
		message_count: 23,
		owner_id: 'u_1',
		created_at: TWO_DAYS,
		last_active_at: TWO_DAYS,
	},
	// 千寻 daemon - 归档
	{
		id: 'sess_daemon_design',
		project_id: 'proj_qianxun_daemon',
		title: 'Daemon 真实化设计',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		status: 'Idle',
		message_count: 12,
		owner_id: 'u_1',
		created_at: LAST_WEEK,
		last_active_at: HOUR_AGO,
	},
	// 千寻 daemon - 完成
	{
		id: 'sess_api_design',
		project_id: 'proj_qianxun_daemon',
		title: 'API Key 管理设计',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		status: 'Idle',
		message_count: 6,
		owner_id: 'u_1',
		created_at: '2026-05-28T10:00:00Z',
		last_active_at: '2026-05-29T15:00:00Z',
	},
	// qianxun-test - 完成
	{
		id: 'sess_eip_test',
		project_id: 'proj_qianxun_test',
		title: 'EIP 测试程序确认',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		status: 'Idle',
		message_count: 5,
		owner_id: 'u_1',
		created_at: '2026-05-25T09:00:00Z',
		last_active_at: '2026-05-26T11:00:00Z',
	},
];
