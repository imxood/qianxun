// qianxun-desktop/src/lib/mock/sub_sessions.ts
// 3 个 sub_session mock (JWT plan 的 3 个 task)

import type { SubSession } from '$lib/types/entity';

const START_DESIGN = '2026-06-07T21:40:00Z';
const END_DESIGN = '2026-06-07T21:52:00Z';
const START_JWT = '2026-06-07T21:52:00Z';
const END_JWT = '2026-06-07T22:00:00Z';
const START_TEST = '2026-06-07T22:00:00Z';

export const mockSubSessions: SubSession[] = [
	// Task 1: 设计 users 表 (done)
	{
		id: 'sub_schema_001',
		plan_id: 'plan_jwt_42',
		plan_task_id: 'task_design_schema',
		parent_session_id: 'sess_jwt_auth',
		role: 'coder',
		status: 'Done',
		messages: [
			{
				id: 'msg_sub_1_1',
				session_id: null,
				sub_session_id: 'sub_schema_001',
				role: 'user',
				content: '设计 users 表 (含 password_hash, created_at), 加索引',
				created_at: START_DESIGN,
			},
			{
				id: 'msg_sub_1_2',
				session_id: null,
				sub_session_id: 'sub_schema_001',
				role: 'assistant',
				content: '好的, 我先看一下现有 schema:\n\n```sql\nCREATE TABLE users (\n  id INTEGER PRIMARY KEY,\n  ...\n)\n```\n\n加 password_hash 和索引:',
				tool_calls: [
					{
						id: 'toolu_1',
						name: 'read_file',
						arguments: { path: 'src/db/schema.sql' },
						result: { content: 'CREATE TABLE...', is_error: false, elapsed_ms: 12 },
					},
				],
				created_at: START_DESIGN,
			},
			{
				id: 'msg_sub_1_3',
				session_id: null,
				sub_session_id: 'sub_schema_001',
				role: 'assistant',
				content: '完成. 新增 users 表 + 索引, 改动在 src/db/schema.sql:38-62.',
				created_at: END_DESIGN,
			},
		],
		output: {
			files_added: ['src/db/schema.sql'],
			lines_added: 25,
			lines_removed: 0,
		},
		started_at: START_DESIGN,
		ended_at: END_DESIGN,
	},
	// Task 2: JWT 签发 (done)
	{
		id: 'sub_jwt_002',
		plan_id: 'plan_jwt_42',
		plan_task_id: 'task_impl_jwt',
		parent_session_id: 'sess_jwt_auth',
		role: 'coder',
		status: 'Done',
		messages: [
			{
				id: 'msg_sub_2_1',
				session_id: null,
				sub_session_id: 'sub_jwt_002',
				role: 'user',
				content: '用 jose 库实现 JWT 签发 (RS256) 与校验',
				created_at: START_JWT,
			},
			{
				id: 'msg_sub_2_2',
				session_id: null,
				sub_session_id: 'sub_jwt_002',
				role: 'assistant',
				content: '好的, 用 jose 库实现:',
				tool_calls: [
					{ id: 'toolu_2', name: 'edit_file', arguments: { path: 'src/auth/jwt.ts' }, result: { content: 'ok', is_error: false, elapsed_ms: 45 } },
					{ id: 'toolu_3', name: 'edit_file', arguments: { path: 'src/routes/login.ts' }, result: { content: 'ok', is_error: false, elapsed_ms: 67 } },
				],
				created_at: END_JWT,
			},
		],
		output: {
			files_added: ['src/auth/jwt.ts', 'src/auth/bcrypt.ts'],
			lines_added: 189,
			lines_removed: 12,
		},
		started_at: START_JWT,
		ended_at: END_JWT,
	},
	// Task 3: 写单测 (running)
	{
		id: 'sub_test_003',
		plan_id: 'plan_jwt_42',
		plan_task_id: 'task_tests',
		parent_session_id: 'sess_jwt_auth',
		role: 'tester',
		status: 'Active',
		messages: [
			{
				id: 'msg_sub_3_1',
				session_id: null,
				sub_session_id: 'sub_test_003',
				role: 'user',
				content: '写 8 个 vitest 用例覆盖认证流程',
				created_at: START_TEST,
			},
			{
				id: 'msg_sub_3_2',
				session_id: null,
				sub_session_id: 'sub_test_003',
				role: 'assistant',
				content: '收到, 先看一下现有测试结构:',
				tool_calls: [
					{ id: 'toolu_4', name: 'grep', arguments: { pattern: 'describe\\(|it\\(' }, result: { content: 'tests/...', is_error: false, elapsed_ms: 8 } },
				],
				created_at: START_TEST,
			},
		],
		output: null,
		started_at: START_TEST,
		ended_at: null,
	},
];
