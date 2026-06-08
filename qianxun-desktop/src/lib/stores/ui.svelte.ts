// qianxun-desktop/src/lib/stores/ui.svelte.ts
// UI 状态 (theme / 列宽 / 当前活跃)
// Mock 阶段: 不持久化, 数据随 session 重新 seed

import { browser } from '$app/environment';

import type { Theme, ActiveView, Toast } from '$lib/types/ui';

function createUiStore() {
	let theme = $state<Theme>('dark');
	let col1Width = $state(260);
	let col3Width = $state(320);
	let col3Collapsed = $state(false);
	const expandedProjectIds = $state(new Set<string>(['proj_qianxun_desktop']));
	let activeView = $state<ActiveView>({
		kind: 'session',
		session_id: 'sess_jwt_auth',
	});
	const toasts = $state<Toast[]>([]);

	return {
		get theme() {
			return theme;
		},
		setTheme(t: Theme) {
			theme = t;
			if (browser) {
				document.documentElement.classList.toggle('dark', t === 'dark');
			}
		},
		toggleTheme() {
			theme = theme === 'dark' ? 'light' : 'dark';
			if (browser) {
				document.documentElement.classList.toggle('dark', theme === 'dark');
			}
		},
		get col1Width() {
			return col1Width;
		},
		setCol1Width(w: number) {
			col1Width = Math.max(180, Math.min(560, w));
		},
		get col3Width() {
			return col3Width;
		},
		setCol3Width(w: number) {
			col3Width = Math.max(180, Math.min(560, w));
		},
		get col3Collapsed() {
			return col3Collapsed;
		},
		toggleCol3Collapsed() {
			col3Collapsed = !col3Collapsed;
		},
		expandCol3() {
			col3Collapsed = false;
		},
		get expandedProjectIds() {
			return expandedProjectIds;
		},
		isProjectExpanded(id: string) {
			return expandedProjectIds.has(id);
		},
		toggleProjectExpanded(id: string) {
			if (expandedProjectIds.has(id)) {
				expandedProjectIds.delete(id);
			} else {
				expandedProjectIds.add(id);
			}
		},
		expandProject(id: string) {
			expandedProjectIds.add(id);
		},
		get activeView() {
			return activeView;
		},
		setActiveView(v: ActiveView) {
			activeView = v;
		},
		switchToSession(session_id: string) {
			activeView = { kind: 'session', session_id };
		},
		switchToSubSession(sub_session_id: string, parent_session_id: string) {
			activeView = { kind: 'sub_session', sub_session_id, parent_session_id };
		},
		switchToNew(project_id: string | null = null) {
			activeView = { kind: 'new', project_id };
		},
		switchToEmpty() {
			activeView = { kind: 'empty' };
		},
		get toasts() {
			return toasts;
		},
		pushToast(toast: Omit<Toast, 'id'>) {
			const id = `toast_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
			const t: Toast = { id, ...toast };
			toasts.push(t);
			if (toast.timeout_ms !== 0) {
				const ms = toast.timeout_ms ?? 5000;
				setTimeout(() => this.dismissToast(id), ms);
			}
		},
		dismissToast(id: string) {
			const i = toasts.findIndex((t) => t.id === id);
			if (i >= 0) toasts.splice(i, 1);
		},
	};
}

export const uiStore = createUiStore();
