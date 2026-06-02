// ───────────────────────────────────────────────────────────────────────────
// sqlite.ts — Stage 5 §11 项目/会话列表本地缓存
// 与 docs/30_子项目规划/03-tauri-desktop.md §11.2 缓存层一致
//
// 目标:
//   - 离线启动时优先回填缓存 (避免 spinner 转 3s 等 daemon)
//   - 网络/Daemon 失败时降级到缓存
//   - 后台静默刷新 (Stage 6 TODO, Stage 5 只读)
//
// 双后端:
//   - Tauri 容器内:  IndexedDB (key-value, 不引 idb-keyval 依赖, 直接用原生 API;
//                    等 Stage 6 接 tauri-plugin-sql 真正 SQLite 时再迁移).
//   - 浏览器 web:    localStorage (json 字符串, 单 key 限 ~5MB, 缓存 < 1MB 足够).
//
// 设计取舍:
//   - 原生 IndexedDB API 异步 + event-based, 用 Promise 包装.
//   - key 命名: 'projects' / 'sessions:<projectId>' (避免单 key 列表过大).
//   - getCached* 返回 null 表示无缓存 (与 empty array 区分).
// ───────────────────────────────────────────────────────────────────────────

import { isTauri } from "$lib/ipc/bridge";
import type { Project, Session } from "$lib/types/ipc";

const DB_NAME = "qianxun-cache";
const DB_VERSION = 1;
const STORE_NAME = "kv";

const LS_PROJECTS_KEY = "qianxun.cache.projects";
const LS_SESSIONS_KEY_PREFIX = "qianxun.cache.sessions.";

// ─── IndexedDB helper (Tauri 模式) ─────────────────────────────────────────

function openDb(): Promise<IDBDatabase> {
	return new Promise((resolve, reject) => {
		if (typeof indexedDB === "undefined") {
			reject(new Error("IndexedDB 不可用 (SSR?)"));
			return;
		}
		const req = indexedDB.open(DB_NAME, DB_VERSION);
		req.onupgradeneeded = () => {
			const db = req.result;
			if (!db.objectStoreNames.contains(STORE_NAME)) {
				db.createObjectStore(STORE_NAME);
			}
		};
		req.onsuccess = () => resolve(req.result);
		req.onerror = () => reject(req.error ?? new Error("IDB open 失败"));
	});
}

async function idbGet<T>(key: string): Promise<T | undefined> {
	const db = await openDb();
	return await new Promise<T | undefined>((resolve, reject) => {
		const tx = db.transaction(STORE_NAME, "readonly");
		const store = tx.objectStore(STORE_NAME);
		const req = store.get(key);
		req.onsuccess = () => resolve(req.result as T | undefined);
		req.onerror = () => reject(req.error);
	});
}

async function idbSet(key: string, value: unknown): Promise<void> {
	const db = await openDb();
	return await new Promise<void>((resolve, reject) => {
		const tx = db.transaction(STORE_NAME, "readwrite");
		const store = tx.objectStore(STORE_NAME);
		const req = store.put(value, key);
		req.onsuccess = () => resolve();
		req.onerror = () => reject(req.error);
	});
}

// ─── 公共 API ─────────────────────────────────────────────────────────────

/// 缓存项目列表. Tauri 走 IndexedDB, web 走 localStorage.
export async function cacheProjects(projects: Project[]): Promise<void> {
	if (isTauri()) {
		await idbSet("projects", projects);
		return;
	}
	try {
		localStorage.setItem(LS_PROJECTS_KEY, JSON.stringify(projects));
	} catch {
		// ignore (quota / private mode)
	}
}

/// 读项目缓存. 失败 / 未缓存 → 返回 null (调用方走网络/Daemon 真实请求).
export async function getCachedProjects(): Promise<Project[] | null> {
	if (isTauri()) {
		try {
			const v = await idbGet<Project[]>("projects");
			return Array.isArray(v) ? v : null;
		} catch {
			return null;
		}
	}
	try {
		const raw = localStorage.getItem(LS_PROJECTS_KEY);
		if (!raw) return null;
		const parsed = JSON.parse(raw) as Project[];
		return Array.isArray(parsed) ? parsed : null;
	} catch {
		return null;
	}
}

/// 缓存会话列表 (按 projectId 分键).
export async function cacheSessions(projectId: string, sessions: Session[]): Promise<void> {
	if (isTauri()) {
		await idbSet(`sessions:${projectId}`, sessions);
		return;
	}
	try {
		localStorage.setItem(LS_SESSIONS_KEY_PREFIX + projectId, JSON.stringify(sessions));
	} catch {
		// ignore
	}
}

/// 读会话缓存. projectId 必传, 避免一个 key 装所有项目过大.
export async function getCachedSessions(projectId: string): Promise<Session[] | null> {
	if (isTauri()) {
		try {
			const v = await idbGet<Session[]>(`sessions:${projectId}`);
			return Array.isArray(v) ? v : null;
		} catch {
			return null;
		}
	}
	try {
		const raw = localStorage.getItem(LS_SESSIONS_KEY_PREFIX + projectId);
		if (!raw) return null;
		const parsed = JSON.parse(raw) as Session[];
		return Array.isArray(parsed) ? parsed : null;
	} catch {
		return null;
	}
}

/// 清除项目缓存 (Stage 6 删除项目时调用). Stage 5 不暴露 UI, 留 API 备.
export async function clearProjectCache(projectId: string): Promise<void> {
	if (isTauri()) {
		const db = await openDb();
		await new Promise<void>((resolve, reject) => {
			const tx = db.transaction(STORE_NAME, "readwrite");
			const req = tx.objectStore(STORE_NAME).delete(`sessions:${projectId}`);
			req.onsuccess = () => resolve();
			req.onerror = () => reject(req.error);
		});
		return;
	}
	try {
		localStorage.removeItem(LS_SESSIONS_KEY_PREFIX + projectId);
	} catch {
		// ignore
	}
}

/// 清除所有缓存 (Settings 页 "清空本地缓存" 按钮调用). Stage 5 留 API.
export async function clearAllCache(): Promise<void> {
	if (isTauri()) {
		const db = await openDb();
		await new Promise<void>((resolve, reject) => {
			const tx = db.transaction(STORE_NAME, "readwrite");
			const req = tx.objectStore(STORE_NAME).clear();
			req.onsuccess = () => resolve();
			req.onerror = () => reject(req.error);
		});
		return;
	}
	try {
		localStorage.removeItem(LS_PROJECTS_KEY);
		const prefix = LS_SESSIONS_KEY_PREFIX;
		for (let i = localStorage.length - 1; i >= 0; i--) {
			const key = localStorage.key(i);
			if (key && key.startsWith(prefix)) localStorage.removeItem(key);
		}
	} catch {
		// ignore
	}
}
