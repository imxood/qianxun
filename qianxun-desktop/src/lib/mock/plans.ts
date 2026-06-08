// qianxun-desktop/src/lib/mock/plans.ts
// 2 个 plan: 1 个 running (JWT auth), 1 个 done (dark mode)

import type { Plan, ChangedFile } from '$lib/types/entity';

const NOW = '2026-06-07T22:00:00Z';
const YESTERDAY = '2026-06-06T15:30:00Z';
const MIN_AGO = '2026-06-07T21:56:00Z';

export const mockPlans: Plan[] = [
	// Running Plan: 实现 JWT 用户认证
	{
		id: 'plan_jwt_42',
		session_id: 'sess_jwt_auth',
		contract: {
			name: '实现 JWT 用户认证',
			description: '为 qianxun-desktop 加 JWT 登录, 用 bcrypt 加密, 含单测',
			timeout_ms: 1800000, // 30 min
			tasks: [
				{
					id: 'task_design_schema',
					title: '设计 users 表 + 索引',
					prompt: '设计 users 表 (含 password_hash, created_at), 加索引, 用 SQL 写出 schema',
					assigned_to: 'coder',
					verified_by: 'code-reviewer',
					verify_prompt: '检查 schema 是否合理, 索引是否覆盖常见查询',
					depends_on: [],
					timeout_ms: 600000, // 10 min
				},
				{
					id: 'task_impl_jwt',
					title: '实现 JWT 签发与校验',
					prompt: '用 jose 库实现 JWT 签发 (RS256) 与校验, 含 access token + refresh token',
					assigned_to: 'coder',
					verified_by: 'code-reviewer',
					verify_prompt: '检查 jose 用法是否正确, 错误处理是否完整',
					depends_on: ['task_design_schema'],
					timeout_ms: 900000, // 15 min
				},
				{
					id: 'task_tests',
					title: '写认证流程单测',
					prompt: '写 8 个 vitest 用例: 正常登录 / 错误密码 / 过期 token / 签名错误 / 用户不存在 / 并发登录 / 登出失效 / 刷新 token',
					assigned_to: 'tester',
					verified_by: 'code-reviewer',
					verify_prompt: '检查测试覆盖率和边界 case',
					depends_on: ['task_impl_jwt'],
					timeout_ms: 900000,
				},
			],
		},
		status: 'Running',
		started_at: '2026-06-07T21:40:00Z',
		ended_at: null,
		result: null,
		attachments: [],
	},
	// Done Plan: dark mode
	{
		id: 'plan_dark_41',
		session_id: 'sess_dark_mode',
		contract: {
			name: '加 dark mode',
			description: '切换 OKLCH 主题变量 + 修对齐',
			timeout_ms: 1800000,
			tasks: [
				{
					id: 'task_dark_1',
					title: '切 OKLCH 主题',
					prompt: '改 layout.css 加 .dark 类变量',
					assigned_to: 'coder',
					verified_by: 'code-reviewer',
					verify_prompt: '',
					depends_on: [],
					timeout_ms: 600000,
				},
			],
		},
		status: 'Done',
		started_at: YESTERDAY,
		ended_at: YESTERDAY,
		result: {
			summary: 'dark mode 切换完成, 修了对齐问题, 写了 5 个测试',
			tasks_completed: 1,
			tasks_total: 1,
			deliverables: [
				'layout.css: 添加 .dark 类 OKLCH 变量',
				'+layout.svelte: theme toggle 按钮',
				'修 Sidebar 顶部 padding',
				'+3 个测试用例 (theme 切换 / localStorage 持久化 / 系统主题检测)',
			],
		},
		attachments: [
			{ name: 'layout.css', kind: 'file', ref: 'src/routes/layout.css' },
			{ name: 'theme-store.test.ts', kind: 'file', ref: 'src/lib/stores/theme.test.ts' },
		],
	},
];

// JWT plan 的变更文件 (mock)
export const mockChangedFiles: ChangedFile[] = [
	{ kind: '+', path: 'src/auth/jwt.ts', task_id: 'task_impl_jwt' },
	{ kind: '+', path: 'src/auth/users.ts', task_id: 'task_design_schema' },
	{ kind: '+', path: 'src/auth/bcrypt.ts', task_id: 'task_impl_jwt' },
	{ kind: '~', path: 'src/routes/login.ts', task_id: 'task_impl_jwt' },
	{ kind: '~', path: 'src/middleware/auth.ts', task_id: 'task_impl_jwt' },
	{ kind: '+', path: 'tests/auth.test.ts', task_id: 'task_tests' },
];
