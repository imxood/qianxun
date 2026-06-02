// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — IPC Bridge (Stage 2)
//
// 抽象 Tauri 2.0 invoke/listen 调用, 让上层 store 不感知运行环境:
//   - 在 Tauri 容器内: 走 @tauri-apps/api 真实 IPC (Rust src-tauri/ 后端)
//   - 在浏览器中 (Web dev / pnpm dev): 走 mock + 浏览器 fetch fallback
//
// Stage 2 范围: 4 个导出函数 (healthCheck / fetchDaemonHealth /
// onDaemonStateChanged / isTauri). 不接 SSE (Stage 3), 不接 Team (Stage 4).
//
// Stage 6a: + setSecret / getSecret (Tauri stronghold 凭据加密, §11.3).
// ───────────────────────────────────────────────────────────────────────────

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { HealthStatus } from "$lib/types/ipc";

/// 检查是否在 Tauri 容器内.
/// Tauri 2.0 在 window 上挂 `__TAURI_INTERNALS__` 内部对象 (非 `__TAURI__`).
/// SSR 场景下没有 window, 一律按 web 处理.
export function isTauri(): boolean {
	if (typeof window === "undefined") return false;
	return "__TAURI_INTERNALS__" in window;
}

/// Invoke Tauri command: `health_check` (本地 mock, 不走网络).
/// 非 Tauri 环境下直接返回 mock offline 状态, 让 UI 能跑出降级态.
export async function healthCheck(): Promise<HealthStatus> {
	if (!isTauri()) {
		return mockHealth();
	}
	return await invoke<HealthStatus>("health_check");
}

/// Invoke Tauri command: 实际 fetch daemon `/v1/system/health` 端点.
/// Tauri 环境: 走 Rust 端 `daemon_health_fetch(url)`, 由 reqwest 3s 超时.
/// Web 环境: 浏览器 fetch (受 CORS 限制; 失败时返回 offline).
export async function fetchDaemonHealth(daemonUrl: string): Promise<HealthStatus> {
	if (!isTauri()) {
		return await webFetchDaemonHealth(daemonUrl);
	}
	return await invoke<HealthStatus>("daemon_health_fetch", { url: daemonUrl });
}

/// Listen Tauri event: `daemon://state-changed` (连接状态机变化).
/// Stage 2: 后端 setup 阶段会立即发一次 'connected' 用来验证 IPC 桥接通.
/// 真实状态机接入留 Stage 3 (与 daemon-stage2-sse-stream 对齐).
/// 非 Tauri 环境返回 noop unlisten 函数, 调用方无需分支.
export async function onDaemonStateChanged(
	handler: (state: string) => void
): Promise<UnlistenFn> {
	if (!isTauri()) {
		// Web 模式下立即 mock 一次 'offline', 让 UI 状态机有一个起点.
		handler("offline");
		return () => {};
	}
	return await listen<string>("daemon://state-changed", (e) => handler(e.payload));
}

/// Invoke Tauri command: `set_secret` (加密存到 stronghold vault, §11.3).
/// 凭据 = API key / VPS access_token / 强密码 hash 等敏感值.
/// Web 模式: localStorage 临时存 (Stage 7 升级为 IndexedDB 加密), 仅 dev 用.
export async function setSecret(
	key: string,
	value: string,
	password: string
): Promise<void> {
	if (!isTauri()) {
		// Web dev fallback: base64 编码到 localStorage (不是真加密, 仅脱敏明文)
		// Stage 7 替换为 IndexedDB 加密.
		localStorage.setItem(`secret-${key}`, btoa(value));
		localStorage.setItem(`secret-${key}-pwd`, btoa(password));
		return;
	}
	await invoke("set_secret", { key, value, password });
}

/// Invoke Tauri command: `get_secret` (从 stronghold vault 解密读取).
/// 凭据不存在 / 密码错 → 返回 null (业务上等价, 用户重输密码即可).
/// Web 模式: 从 localStorage 读 base64 解码.
export async function getSecret(
	key: string,
	password: string
): Promise<string | null> {
	if (!isTauri()) {
		const v = localStorage.getItem(`secret-${key}`);
		if (!v) return null;
		// 验证 password (粗校验, 不防篡改 — 仅 dev 调试场景)
		const storedPwd = localStorage.getItem(`secret-${key}-pwd`);
		if (!storedPwd || atob(storedPwd) !== password) return null;
		return atob(v);
	}
	return await invoke<string | null>("get_secret", { key, password });
}

/// Stage 10b: Invoke Tauri command: `delete_secret` (从 stronghold vault 删除).
/// 业务用途: 用户换 VPS access_token / 撤权 API key 时, 删 vault 里的旧值.
/// 凭据不存在 → 静默成功 (idempotent, 跟 stronghold store().delete() 语义对齐).
/// Web 模式: 从 localStorage 删 base64 项 + password 项.
export async function deleteSecret(
	key: string,
	password: string
): Promise<boolean> {
	if (!isTauri()) {
		// 验证 password (跟 getSecret 一致的安全检查)
		const storedPwd = localStorage.getItem(`secret-${key}-pwd`);
		if (!storedPwd || atob(storedPwd) !== password) return false;
		const existed = localStorage.getItem(`secret-${key}`) !== null;
		localStorage.removeItem(`secret-${key}`);
		localStorage.removeItem(`secret-${key}-pwd`);
		return existed;
	}
	return await invoke<boolean>("delete_secret", { key, password });
}

// ─── 内部 helpers ────────────────────────────────────────────────────────

async function webFetchDaemonHealth(daemonUrl: string): Promise<HealthStatus> {
	const endpoint = `${daemonUrl.replace(/\/$/, "")}/v1/system/health`;
	try {
		const r = await fetch(endpoint, { signal: AbortSignal.timeout(3000) });
		if (!r.ok) return mockOffline();
		// 尝试按后端 HealthStatus 格式解析; 失败时降级为 offline.
		const data = (await r.json()) as Partial<HealthStatus>;
		return {
			status: data.status ?? "offline",
			version: data.version ?? "web",
			uptime_sec: data.uptime_sec ?? 0,
			session_count: data.session_count ?? 0,
			mcp_online: data.mcp_online ?? 0,
			provider_status: data.provider_status ?? {},
		};
	} catch {
		return mockOffline();
	}
}

function mockHealth(): HealthStatus {
	return {
		status: "offline",
		version: "mock",
		uptime_sec: 0,
		session_count: 0,
		mcp_online: 0,
		provider_status: {},
	};
}

function mockOffline(): HealthStatus {
	return {
		status: "offline",
		version: "unknown",
		uptime_sec: 0,
		session_count: 0,
		mcp_online: 0,
		provider_status: {},
	};
}
