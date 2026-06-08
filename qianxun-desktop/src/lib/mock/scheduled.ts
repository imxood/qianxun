// qianxun-desktop/src/lib/mock/scheduled.ts
// 1 个定时任务 mock (Col 1 顶部)

import type { ScheduledTask } from '$lib/types/entity';

export const mockScheduledTasks: ScheduledTask[] = [
	{
		id: 'sched_memory_001',
		name: '记忆维护',
		kind: 'memory_maintenance',
		cron: '0 3 * * *',
		enabled: true,
		last_run_at: '2026-06-07T03:00:00Z',
		next_run_at: '2026-06-08T03:00:00Z',
	},
];
