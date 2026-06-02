// ───────────────────────────────────────────────────────────────────────────
// VpsStore — VPS (WebSocket) 接入骨架 (Stage 4)
// 与 docs/30_子项目规划/03-tauri-desktop.md §10.4 完全一致.
//
// Stage 4 简化:
//   - 只验证 WS 连接 (不接 team 业务路由, 不接邀请推送)
//   - VPS URL 从 localStorage 读取 (vpsUrl), 启动时尝试连接
//   - 失败不报错 (VPS 是可选的, 用户只连本地 Daemon 也行)
//   - 30s 周期重试 (与 §10.4 文档一致)
//
// 状态机 (3 态):
//   'offline'     — vpsUrl 为空 或 上次连接失败
//   'connecting'  — 正在尝试 WS handshake
//   'connected'   — WS 握手成功
//
// Stage 6b: + 团队/项目写操作 (inviteMember / changeRole / assignProject).
//   - 当前为本地 mock: 不发真实 fetch, 只更新 `teamMembers` / `projectAssignees`
//     状态, 让 UI 能验证写操作流程. Stage 6c 替换为真实 VPS REST fetch.
//   - 真实 fetch 时, 凭据取自 settingsStore.getVpsToken() (Stage 6b 简化: 从
//     localStorage 读明文, Stage 7 升级为 stronghold + 密码弹窗).
// ───────────────────────────────────────────────────────────────────────────

import type { TeamMember, TeamRole } from "$lib/types/ipc";

const STORAGE_KEY = "qianxun.vps.url";
const VPS_PING_INTERVAL_MS = 30_000; // §10.4
const VPS_HANDSHAKE_TIMEOUT_MS = 5_000;
// Stage 6b: mock 写操作的人为延迟, 让 UI 能看到 "提交中…" 状态. Stage 6c 真发
// 请求时这个延迟自然来自网络, 不需要模拟.
const MOCK_WRITE_DELAY_MS = 50;

export type VpsConnectionState = "offline" | "connecting" | "connected";

class VpsStore {
	vpsUrl = $state<string>("");
	connectionState = $state<VpsConnectionState>("offline");
	lastError = $state<string | null>(null);
	lastConnectedAt = $state<number | null>(null);

	// Stage 6b: mock 写操作的本地状态. 真实接入后会被 fetch + 缓存策略取代.
	//  - teamMembers:      per-team 成员列表 (mock 后端 truth)
	//  - projectAssignees: per-project 已分配成员 id 列表
	teamMembers = $state<Record<string, TeamMember[]>>({});
	projectAssignees = $state<Record<string, string[]>>({});

	#timer: ReturnType<typeof setInterval> | null = null;
	#ws: WebSocket | null = null;

	// ─── 派生 ────────────────────────────────────────────────────────────────

	get isDegraded(): boolean {
		// VPS 是可选的 — "降级" 仅在用户配过 URL 但连不上时为 true;
		// 未配置 (vpsUrl 为空) 不算降级 (用户根本没打算用).
		return this.vpsUrl.length > 0 && this.connectionState !== "connected";
	}

	// ─── 配置 URL ────────────────────────────────────────────────────────────

	/// 用户在 Settings 页填入 VPS URL 后调用. 自动触发一次连接尝试.
	setVpsUrl(url: string): void {
		this.vpsUrl = url.trim();
		try {
			localStorage.setItem(STORAGE_KEY, this.vpsUrl);
		} catch {
			// ignore (private mode)
		}
		// 改了 URL 立即试一次 (如果有 timer 在跑会忽略自己)
		void this.connect();
	}

	/// 启动周期 ping. 可重复调用 (内部去重).
	startHealthCheck(): void {
		// 启动时从 storage 读 URL
		try {
			const raw = localStorage.getItem(STORAGE_KEY);
			if (raw) this.vpsUrl = raw;
		} catch {
			// ignore
		}

		if (this.#timer) return;
		void this.connect(); // 立即试一次
		this.#timer = setInterval(() => void this.connect(), VPS_PING_INTERVAL_MS);
	}

	stopHealthCheck(): void {
		if (this.#timer) {
			clearInterval(this.#timer);
			this.#timer = null;
		}
		try {
			this.#ws?.close();
		} catch {
			// ignore
		}
		this.#ws = null;
		this.connectionState = "offline";
	}

	// ─── 内部: 实际 WS 握手 ──────────────────────────────────────────────────

	/// 试一次 WS 握手. 成功 → 'connected' 5s 后立即关 (健康检查不保持长连接,
	/// 真实长连接留 Stage 5). 失败 → 'offline' + lastError.
	async connect(): Promise<void> {
		if (!this.vpsUrl) {
			this.connectionState = "offline";
			return;
		}
		// SSR / 非浏览器环境 — 直接退出
		if (typeof WebSocket === "undefined") {
			this.connectionState = "offline";
			this.lastError = "WebSocket API 不可用 (SSR?)";
			return;
		}

		// 关掉旧 socket
		try {
			this.#ws?.close();
		} catch {
			// ignore
		}
		this.#ws = null;

		this.connectionState = "connecting";
		this.lastError = null;

		const url = this.normalizeUrl(this.vpsUrl);

		try {
			const ws = new WebSocket(url);
			this.#ws = ws;

			const timeout = setTimeout(() => {
				try {
					ws.close();
				} catch {
					// ignore
				}
				if (this.connectionState === "connecting") {
					this.connectionState = "offline";
					this.lastError = "握手超时 (5s)";
				}
			}, VPS_HANDSHAKE_TIMEOUT_MS);

			ws.addEventListener(
				"open",
				() => {
					clearTimeout(timeout);
					if (this.#ws !== ws) return; // 已被新连接替换
					this.connectionState = "connected";
					this.lastConnectedAt = Date.now();
					// 健康检查模式: 5s 后主动关. 真实长连接留 Stage 5.
					setTimeout(() => {
						try {
							ws.close();
						} catch {
							// ignore
						}
						if (this.#ws === ws) {
							this.#ws = null;
							this.connectionState = "offline";
						}
					}, 5_000);
				},
				{ once: true }
			);

			ws.addEventListener(
				"error",
				() => {
					clearTimeout(timeout);
					if (this.#ws !== ws) return;
					this.connectionState = "offline";
					this.lastError = "WebSocket 握手失败";
				},
				{ once: true }
			);

			ws.addEventListener(
				"close",
				() => {
					clearTimeout(timeout);
					if (this.#ws !== ws) return;
					this.#ws = null;
					if (this.connectionState === "connecting") {
						this.connectionState = "offline";
						this.lastError = this.lastError ?? "连接被关闭";
					} else if (this.connectionState === "connected") {
						// 服务端主动关 (我们 5s 后自关也会触发)
						this.connectionState = "offline";
					}
				},
				{ once: true }
			);
		} catch (e) {
			this.connectionState = "offline";
			this.lastError = (e as Error).message || "WebSocket 构造失败";
		}
	}

	/// 'https://vps.example.com' → 'wss://vps.example.com/hub'
	/// 'http://...' → 'ws://...'
	/// 已有 ws:// 或 wss:// 直接返回.
	normalizeUrl(url: string): string {
		let u = url.trim();
		if (!u) return u;
		if (u.startsWith("ws://") || u.startsWith("wss://")) {
			return u.endsWith("/hub") ? u : `${u.replace(/\/$/, "")}/hub`;
		}
		if (u.startsWith("http://")) {
			u = "ws://" + u.slice("http://".length);
		} else if (u.startsWith("https://")) {
			u = "wss://" + u.slice("https://".length);
		}
		return u.endsWith("/hub") ? u : `${u.replace(/\/$/, "")}/hub`;
	}

	// ─── Stage 6b: 团队/项目写操作 (mock) ────────────────────────────────────
	//
	// 三个方法当前都是本地 mock, 不发网络请求, 仅更新 store 内部状态.
	// Stage 6c 将替换为真实 VPS REST fetch (POST /api/teams/:id/members 等).
	// 真实 fetch 时, 凭据 (vpsToken) 取自 settingsStore.getVpsToken().

	/// POST /api/teams/:teamId/members { user_id, display_name, role }
	/// mock: 50ms 延迟后把新成员追加到 teamMembers[teamId].
	async inviteMember(
		teamId: string,
		userId: string,
		displayName: string,
		role: TeamRole
	): Promise<void> {
		await new Promise((r) => setTimeout(r, MOCK_WRITE_DELAY_MS));
		if (!teamId || !userId) {
			throw new Error("inviteMember: teamId/userId 必填");
		}
		const list = this.teamMembers[teamId] ?? [];
		if (list.some((m) => m.user_id === userId)) {
			throw new Error(`user_id=${userId} 已在团队中`);
		}
		this.teamMembers[teamId] = [
			...list,
			{
				user_id: userId,
				display_name: displayName || userId,
				role,
				joined_at: new Date().toISOString(),
			},
		];
	}

	/// PATCH /api/teams/:teamId/members/:userId { role }
	/// mock: 找到该 user 并改 role.
	async changeRole(teamId: string, userId: string, role: TeamRole): Promise<void> {
		await new Promise((r) => setTimeout(r, MOCK_WRITE_DELAY_MS));
		const list = this.teamMembers[teamId];
		if (!list) {
			throw new Error(`changeRole: team=${teamId} 没有成员`);
		}
		const idx = list.findIndex((m) => m.user_id === userId);
		if (idx < 0) {
			throw new Error(`changeRole: user=${userId} 不在团队中`);
		}
		this.teamMembers[teamId] = list.map((m) =>
			m.user_id === userId ? { ...m, role } : m
		);
	}

	/// POST /api/projects/:projectId/assign { user_id }
	/// mock: 把 userId 加入 projectAssignees[projectId] (去重).
	async assignProject(projectId: string, userId: string): Promise<void> {
		await new Promise((r) => setTimeout(r, MOCK_WRITE_DELAY_MS));
		if (!projectId || !userId) {
			throw new Error("assignProject: projectId/userId 必填");
		}
		const list = this.projectAssignees[projectId] ?? [];
		if (list.includes(userId)) return; // 幂等
		this.projectAssignees[projectId] = [...list, userId];
	}

	/// Stage 6b 测试钩子: 清空 mock 状态. Stage 6c 删除.
	__resetMockState(): void {
		this.teamMembers = {};
		this.projectAssignees = {};
	}
}

export const vpsStore = new VpsStore();
