<script lang="ts">
	import { onMount } from "svelte";
	import ThreeColumnLayout from "$lib/components/layout/ThreeColumnLayout.svelte";
	import Sidebar from "$lib/components/layout/Sidebar.svelte";
	import SessionList from "$lib/components/layout/SessionList.svelte";
	import ChatView from "$lib/components/layout/ChatView.svelte";
	import type { Project, Session, Team } from "$lib/types/ipc";
	import { healthCheck, isTauri, onDaemonStateChanged } from "$lib/ipc/bridge";

	// ─── Stage 1 mock 数据 ──────────────────────────────────────────────────
	// 真实数据 Stage 2 通过 daemon_list_projects / daemon_list_sessions
	// 经 Tauri invoke 获取.
	const mockTeams: Team[] = [
		{
			id: "team_1",
			name: "千寻 R&D",
			created_at: "2026-06-01T08:00:00Z",
			members: [
				{
					user_id: "u_1",
					display_name: "maxu",
					email: "maxu@example.com",
					role: "owner",
					joined_at: "2026-06-01T08:00:00Z",
				},
				{
					user_id: "u_2",
					display_name: "alice",
					email: "alice@example.com",
					role: "developer",
					joined_at: "2026-06-01T09:00:00Z",
				},
			],
		},
	];

	const mockProjects: Project[] = [
		{
			id: "proj_1",
			name: "qianxun",
			path: "E:/git/maxu/qianxun",
			owner_id: "u_1",
			team_id: "team_1",
			created_at: "2026-06-01T08:30:00Z",
		},
		{
			id: "proj_2",
			name: "qianxun-desktop",
			path: "E:/git/maxu/qianxun/qianxun-desktop",
			description: "Tauri 桌面端 (Stage 1 脚手架)",
			owner_id: "u_1",
			team_id: "team_1",
			created_at: "2026-06-01T10:00:00Z",
		},
	];

	const mockSessions: Session[] = [
		{
			id: "sess_1",
			project_id: "proj_1",
			title: "Daemon 真实化设计",
			model: "deepseek-v4-flash",
			status: "active",
			owner_id: "u_1",
			created_at: "2026-06-01T11:00:00Z",
			last_active_at: new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString(),
			message_count: 12,
		},
		{
			id: "sess_2",
			project_id: "proj_1",
			title: "TypeScript 状态管理对比",
			model: "deepseek-v4-flash",
			status: "idle",
			owner_id: "u_1",
			created_at: "2026-06-01T05:00:00Z",
			last_active_at: new Date(Date.now() - 5 * 60 * 60 * 1000).toISOString(),
			message_count: 28,
		},
		{
			id: "sess_3",
			project_id: "proj_2",
			title: "Tauri Stage 1 脚手架",
			model: "deepseek-v4-flash",
			status: "active",
			owner_id: "u_1",
			created_at: "2026-06-02T00:00:00Z",
			last_active_at: new Date(Date.now() - 5 * 60 * 1000).toISOString(),
			message_count: 4,
		},
	];

	// Stage 1 简单选择状态 (Stage 2 用 Svelte 5 store 替代)
	let activeProjectId = $state<string>(mockProjects[0]?.id ?? "");
	let activeSessionId = $state<string | null>(mockSessions[0]?.id ?? null);

	const activeSession = $derived(
		mockSessions.find((s) => s.id === activeSessionId) ?? null
	);

	function onSelectProject(id: string) {
		activeProjectId = id;
		// 默认选中该项目的第一个 session
		const first = mockSessions.find((s) => s.project_id === id);
		activeSessionId = first?.id ?? null;
	}

	function onSelectSession(id: string) {
		activeSessionId = id;
	}

	// ─── Stage 2: IPC 桥接验证 ──────────────────────────────────────────────
	// onMount: 调一次 healthCheck 验证 IPC 桥接通 (Tauri 走 invoke, Web 走 mock),
	// 同时订阅 daemon://state-changed 事件, 让 console 能看到 IPC 消息来回.
	onMount(() => {
		console.log(`[qianxun-desktop] Stage 2 IPC bridge: isTauri=${isTauri()}`);
		void healthCheck()
			.then((status) => {
				console.log("[qianxun-desktop] healthCheck →", status);
			})
			.catch((err) => {
				console.error("[qianxun-desktop] healthCheck failed:", err);
			});

		let unlisten: (() => void) | undefined;
		void onDaemonStateChanged((state) => {
			console.log("[qianxun-desktop] daemon://state-changed →", state);
		}).then((u) => {
			unlisten = u;
		});

		return () => {
			unlisten?.();
		};
	});
</script>

<ThreeColumnLayout>
	{#snippet sidebar()}
		<Sidebar
			projects={mockProjects}
			teams={mockTeams}
			activeProjectId={activeProjectId}
			onSelectProject={onSelectProject}
		/>
	{/snippet}

	{#snippet sessions()}
		<SessionList
			sessions={mockSessions.filter((s) => !activeProjectId || s.project_id === activeProjectId)}
			activeSessionId={activeSessionId ?? undefined}
			onSelectSession={onSelectSession}
		/>
	{/snippet}

	<ChatView {activeSession} />
</ThreeColumnLayout>
