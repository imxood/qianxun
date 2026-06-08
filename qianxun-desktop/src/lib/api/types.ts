// qianxun-desktop/src/lib/api/types.ts
// 跟 qianxun/src/daemon/ui/src/lib/types/chat.ts 对齐
// (注: v0.2 阶段, 4a-2 切 v1.0 时再扩)

export interface ChatSessionCreated {
	session: ChatSession;
}

export interface ChatSessionList {
	sessions: ChatSession[];
}

export interface ChatSession {
	id: string;
	title: string;
	project_id: string | null;
	model: string;
	status: 'Active' | 'Idle' | 'Archived';
	created_at: string;
	last_active_at: string;
}
