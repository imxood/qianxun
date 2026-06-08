// qianxun-desktop/src/lib/mock/experience.ts
// 3 条项目经验 mock (走 qianxun-memory, mock 阶段 localStorage)

import type { ProjectExperience } from '$lib/types/entity';

export const mockExperience: ProjectExperience[] = [
	{
		id: 'exp_001',
		project_id: 'proj_qianxun_desktop',
		content: '本项目用 Svelte 5 runes 模式 ($state / $derived / $effect), 不用旧 store API. 跨组件共享用 createXxxStore() 函数模式.',
		source_session_id: 'sess_dark_mode',
		tags: ['svelte', 'runes', 'frontend'],
		created_at: '2026-06-06T16:00:00Z',
	},
	{
		id: 'exp_002',
		project_id: 'proj_qianxun_desktop',
		content: 'Tailwind v4 用 CSS-first config (在 layout.css 里定义 OKLCH 变量 + @theme), 不用 tailwind.config.js 的 JS config.',
		source_session_id: 'sess_dark_mode',
		tags: ['tailwind', 'css'],
		created_at: '2026-06-06T16:01:00Z',
	},
	{
		id: 'exp_003',
		project_id: 'proj_qianxun_desktop',
		content: 'shadcn-svelte 组件从 $lib/components/ui 引用, 别名 $ui 配在 components.json.',
		tags: ['shadcn', 'ui'],
		created_at: '2026-06-06T16:02:00Z',
	},
];
