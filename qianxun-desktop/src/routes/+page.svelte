<script lang="ts">
	import { onMount } from "svelte";
	import ThreeColumnLayout from "$lib/components/layout/ThreeColumnLayout.svelte";
	import Sidebar from "$lib/components/layout/Sidebar.svelte";
	import SessionList from "$lib/components/layout/SessionList.svelte";
	import ChatView from "$lib/components/layout/ChatView.svelte";
	import ConnectionBanner from "$lib/components/chat/ConnectionBanner.svelte";
	import type { Project, Session, Team } from "$lib/types/ipc";
	import { healthCheck, isTauri, onDaemonStateChanged } from "$lib/ipc/bridge";
	import { connectionStore } from "$lib/stores/connection.svelte";
	import { sessionStore } from "$lib/stores/session.svelte";
	import { vpsStore } from "$lib/stores/vps.svelte";
	import { teamStore } from "$lib/stores/team.svelte";

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

		// ─── Stage 4: 离线队列启动时回填 + VPS 周期 ping ────────────────────
		sessionStore.loadOfflineQueue();
		if (sessionStore.offlineQueueSize > 0) {
			console.info(
				`[+page] 启动时回填 ${sessionStore.offlineQueueSize} 条离线消息`
			);
		}
		vpsStore.startHealthCheck();

		// Stage 6c: 用 mock 数据 seed teamStore (Sidebar 集成路由).
		// 真实数据由 teamStore.refresh() 在 VPS 在线时拉取.
		teamStore.seed(mockTeams, mockProjects);

		// Stage 4 §10.3: 周期性检查 — daemon 从 degraded 变 connected 时
		// 自动 flush 离线队列. 用 setInterval 不用 $effect, 避免 Svelte 5
		// read-write effect cycle 陷阱 (写 sessionStore.offlineQueue 会进入
		// 反应链; 改成"读 connectionStore.daemonState → 调 flush"是单向).
		const flushTimer = setInterval(() => {
			if (connectionStore.daemonState === "connected" && sessionStore.offlineQueueSize > 0) {
				void sessionStore.flushOfflineQueue();
			}
		}, 5_000);

		return () => {
			unlisten?.();
			vpsStore.stopHealthCheck();
			clearInterval(flushTimer);
		};
	});
</script>

<ThreeColumnLayout>
	{#snippet sidebar()}
		<Sidebar
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

	<div class="flex h-full flex-col gap-2">
		<ConnectionBanner />
		<div class="flex-1 overflow-hidden">
			<ChatView {activeSession} />
		</div>
	</div>
</ThreeColumnLayout>
