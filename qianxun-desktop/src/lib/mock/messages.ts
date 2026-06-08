// qianxun-desktop/src/lib/mock/messages.ts
// 主会话消息 (跟 mockPlans 的 running plan 配套)

import type { Message } from '$lib/types/entity';

const T_21_35 = '2026-06-07T21:35:00Z';
const T_21_36 = '2026-06-07T21:36:00Z';
const T_21_40 = '2026-06-07T21:40:00Z';
const T_21_41 = '2026-06-07T21:41:00Z';
const T_22_00 = '2026-06-07T22:00:00Z';

export const mockMessages: Message[] = [
	// JWT auth session 的消息
	{
		id: 'msg_1',
		session_id: 'sess_jwt_auth',
		sub_session_id: null,
		role: 'user',
		content: '帮我加个 JWT 登录, 用 bcrypt 加密. 记得加测试.',
		created_at: T_21_35,
	},
	{
		id: 'msg_2',
		session_id: 'sess_jwt_auth',
		sub_session_id: null,
		role: 'assistant',
		content: '好的. 这涉及 schema 变更、API 跟单测, 我先拉个 Plan 拆 3 个子任务, 串行跑.',
		created_at: T_21_36,
		plan_ref: 'plan_jwt_42',
	},
	// Plan 块本身 (chat-first §5.2 的设计: Plan 块是 assistant message 的 tool_calls)
	// 这里我们用 plan_ref 标识, 实际渲染时由 PlanBlock 组件接管
	{
		id: 'msg_3',
		session_id: 'sess_jwt_auth',
		sub_session_id: null,
		role: 'assistant',
		content: '',
		plan_ref: 'plan_jwt_42',
		created_at: T_21_40,
	},
	// Plan 跑了一段时间, 主 Agent 持续输出
	{
		id: 'msg_4',
		session_id: 'sess_jwt_auth',
		sub_session_id: null,
		role: 'assistant',
		content: '用户表已建好, JWT 工具函数就位. tester 正在跑 8 个测试用例 (含 3 个边界 case)...',
		created_at: T_22_00,
		streaming: true,
	},
];

// dark mode session (已完成) 的消息, 用于场景 2
export const mockMessagesDarkMode: Message[] = [
	{
		id: 'msg_dm_1',
		session_id: 'sess_dark_mode',
		sub_session_id: null,
		role: 'user',
		content: '顺便帮我加个 dark mode',
		created_at: '2026-06-06T15:00:00Z',
	},
	{
		id: 'msg_dm_2',
		session_id: 'sess_dark_mode',
		sub_session_id: null,
		role: 'assistant',
		content: 'Plan 跑完了, 全部 3 个子任务 done. 改动如下:',
		created_at: '2026-06-06T15:30:00Z',
		plan_ref: 'plan_dark_41',
	},
	{
		id: 'msg_dm_3',
		session_id: 'sess_dark_mode',
		sub_session_id: null,
		role: 'assistant',
		content: '',
		plan_ref: 'plan_dark_41', // Plan 完成卡 (跟 chat-first §5.2 完成态)
		created_at: '2026-06-06T15:30:00Z',
	},
	{
		id: 'msg_dm_4',
		session_id: 'sess_dark_mode',
		sub_session_id: null,
		role: 'assistant',
		content: '要把这次的关键决策沉淀到项目经验吗? (jose / bcrypt rounds=12 / RS256)',
		created_at: '2026-06-06T15:31:00Z',
	},
];
