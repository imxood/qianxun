// qianxun-desktop/src/lib/stores/session.svelte.ts
// Session store (主会话)

import { buildSeed } from '$lib/mock/seed';
import { uiStore } from './ui.svelte';
import type { Session, Message } from '$lib/types/entity';

function createSessionStore() {
	const seed = buildSeed();
	const sessions = $state<Session[]>(seed.sessions);
	const messages = $state<Record<string, Message[]>>({
		sess_jwt_auth: seed.messages,
		sess_dark_mode: seed.messages_dark_mode,
	});

	const activeSession = $derived(
		uiStore.activeView.kind === 'session'
			? sessions.find((s) => s.id === uiStore.activeView.session_id) ?? null
			: null,
	);

	const activeMessages = $derived(activeSession ? messages[activeSession.id] ?? [] : []);

	return {
		get all() {
			return sessions;
		},
		get(id: string): Session | undefined {
			return sessions.find((s) => s.id === id);
		},
		byProject(projectId: string | null): Session[] {
			if (projectId === null) return sessions.filter((s) => s.project_id === null);
			return sessions.filter((s) => s.project_id === projectId);
		},
		get active(): Session | null {
			return activeSession;
		},
		get activeMessages(): Message[] {
			return activeMessages;
		},
		create(opts: {
			project_id: string | null;
			folder?: string | null;
			title?: string;
			provider?: string;
			model?: string;
		}): Session {
			const now = new Date().toISOString();
			const id = `sess_${now.replace(/[-:T.Z]/g, '').slice(0, 17)}_${Math.random().toString(36).slice(2, 8)}`;
			const newSession: Session = {
				id,
				project_id: opts.project_id,
				title: opts.title ?? '新会话',
				provider: opts.provider ?? 'deepseek',
				model: opts.model ?? 'deepseek-v4-flash',
				status: 'Active',
				message_count: 0,
				owner_id: 'u_1',
				created_at: now,
				last_active_at: now,
			};
			sessions.push(newSession);
			messages[id] = [];
			return newSession;
		},
		switchTo(id: string) {
			const s = sessions.find((x) => x.id === id);
			if (s) {
				s.last_active_at = new Date().toISOString();
				uiStore.switchToSession(id);
			}
		},
		getMessages(sessionId: string): Message[] {
			return messages[sessionId] ?? [];
		},
		appendMessage(sessionId: string, msg: Message) {
			if (!messages[sessionId]) messages[sessionId] = [];
			messages[sessionId].push(msg);
			const s = sessions.find((x) => x.id === sessionId);
			if (s) {
				s.message_count = messages[sessionId].length;
				s.last_active_at = msg.created_at;
			}
		},
		countByProject(projectId: string): number {
			return sessions.filter((s) => s.project_id === projectId).length;
		},
	};
}

export const sessionStore = createSessionStore();
