// qianxun-desktop/src/lib/types/ui.ts
// UI 状态 (theme / 列宽 / 当前活跃)

export type Theme = 'dark' | 'light';

export interface ColumnWidths {
	col1: number; // px, 默认 260
	col3: number; // px, 默认 320
}

// SPA 模式下的视图状态 (不依赖 router)
export type ActiveView =
	| { kind: 'session'; session_id: string }
	| { kind: 'sub_session'; sub_session_id: string; parent_session_id: string }
	| { kind: 'new'; project_id: string | null } // 新会话 (含首次空白 Chat)
	| { kind: 'empty' }; // 无 session 的空状态

// Toast 类型
export interface Toast {
	id: string;
	kind: 'info' | 'success' | 'warn' | 'error';
	title: string;
	description?: string;
	timeout_ms?: number; // 0 = 不自动关闭
	action?: {
		label: string;
		href?: string;
		on_click?: () => void;
	};
}
