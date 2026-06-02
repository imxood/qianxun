// ───────────────────────────────────────────────────────────────────────────
// VpsStore — VPS 接入 (Stage 4 + Stage 6c 真接 REST fetch)
//
// Stage 4 范围:
//   - WS 健康检查 (vpsUrl + connectionState 3 态机, 30s 周期重试)
//   - normalizeUrl / setVpsUrl / startHealthCheck / stopHealthCheck
//
// Stage 6b → 6c 演进:
//   - 6b: 3 个写操作 (inviteMember / changeRole / assignProject) 为本地 mock
//   - 6c: 替换为真实 VPS REST fetch (POST /api/teams/:id/members 等),
//          自动带 Authorization: Bearer <vpsToken>. token 取自
//          settingsStore.getVpsToken() (Stage 6b: localStorage 明文, 7 升
//          stronghold + 密码弹窗).
//   - 本地状态 (teamMembers / projectAssignees) 已迁出, 由 teamStore 持有
//     并通过 refresh() 从 VPS 拉取. vpsStore 仅保留写操作 API.
//
// 约束:
//   - 不做错误重试 (Stage 7)
//   - 不做实时 WS 推送同步 (Stage 7)
//   - 不做项目创建 UI (Stage 7)
// ───────────────────────────────────────────────────────────────────────────

import type { TeamRole } from "$lib/types/ipc";
import { settingsStore } from "$lib/stores/settings.svelte";

const STORAGE_KEY = "qianxun.vps.url";
const VPS_PING_INTERVAL_MS = 30_000;
const VPS_HANDSHAKE_TIMEOUT_MS = 5_000;

export type VpsConnectionState = "offline" | "connecting" | "connected";

class VpsStore {
	vpsUrl = $state<string>("");
	connectionState = $state<VpsConnectionState>("offline");
	lastError = $state<string | null>(null);
	lastConnectedAt = $state<number | null>(null);

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
		void this.connect();
	}

	/// 启动周期 ping. 可重复调用 (内部去重).
	startHealthCheck(): void {
		try {
			const raw = localStorage.getItem(STORAGE_KEY);
			if (raw) this.vpsUrl = raw;
		} catch {
			// ignore
		}

		if (this.#timer) return;
		void this.connect();
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

	async connect(): Promise<void> {
		if (!this.vpsUrl) {
			this.connectionState = "offline";
			return;
		}
		if (typeof WebSocket === "undefined") {
			this.connectionState = "offline";
			this.lastError = "WebSocket API 不可用 (SSR?)";
			return;
		}

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
					if (this.#ws !== ws) return;
					this.connectionState = "connected";
					this.lastConnectedAt = Date.now();
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

	// ─── Stage 6c: 写操作 (真接 fetch) ──────────────────────────────────────
	//
	// 三个方法都通过 vpsFetch() 发真实 HTTP 请求, 自动带 Bearer token.
	// 状态由调用方 (通常是 teamStore.refresh() 或组件 onChanged 回调) 拉新.
	// 不做错误重试 / 不做 WS 推送同步 (Stage 7 加).

	/// POST /api/teams/:teamId/members { user_id, display_name, role }
	async inviteMember(
		teamId: string,
		userId: string,
		displayName: string,
		role: TeamRole
	): Promise<void> {
		if (!teamId || !userId) {
			throw new Error("inviteMember: teamId/userId 必填");
		}
		const r = await vpsFetch(`/api/teams/${encodeURIComponent(teamId)}/members`, {
			method: "POST",
			body: JSON.stringify({
				user_id: userId,
				display_name: displayName || userId,
				role,
			}),
		});
		if (!r.ok) {
			throw new Error(`inviteMember failed: HTTP ${r.status} ${r.statusText}`);
		}
	}

	/// PATCH /api/teams/:teamId/members/:userId { role }
	async changeRole(teamId: string, userId: string, role: TeamRole): Promise<void> {
		if (!teamId || !userId) {
			throw new Error("changeRole: teamId/userId 必填");
		}
		const r = await vpsFetch(
			`/api/teams/${encodeURIComponent(teamId)}/members/${encodeURIComponent(userId)}`,
			{
				method: "PATCH",
				body: JSON.stringify({ role }),
			}
		);
		if (!r.ok) {
			throw new Error(`changeRole failed: HTTP ${r.status} ${r.statusText}`);
		}
	}

	/// POST /api/projects/:projectId/assign { user_id }
	async assignProject(projectId: string, userId: string): Promise<void> {
		if (!projectId || !userId) {
			throw new Error("assignProject: projectId/userId 必填");
		}
		const r = await vpsFetch(
			`/api/projects/${encodeURIComponent(projectId)}/assign`,
			{
				method: "POST",
				body: JSON.stringify({ user_id: userId }),
			}
		);
		if (!r.ok) {
			throw new Error(`assignProject failed: HTTP ${r.status} ${r.statusText}`);
		}
	}
}

/// 通用 fetch wrapper, 自动带 Bearer token + JSON headers.
/// token 从 settingsStore.getVpsToken() 取 (Stage 6b: localStorage 明文,
/// Stage 7 替换为 stronghold + 密码弹窗).
async function vpsFetch(path: string, opts: RequestInit = {}): Promise<Response> {
	const base = settingsStore.vpsUrl.trim();
	if (!base) {
		throw new Error("vpsFetch: settingsStore.vpsUrl 未配置");
	}
	const token = settingsStore.getVpsToken();
	return fetch(`${base}${path}`, {
		...opts,
		headers: {
			Authorization: token ? `Bearer ${token}` : "",
			"Content-Type": "application/json",
			...(opts.headers ?? {}),
		},
	});
}

export const vpsStore = new VpsStore();
