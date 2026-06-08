// qianxun-desktop/src/lib/stores/persist.svelte.ts
// localStorage 读写工具 (mock 阶段: 不持久化, 真实化阶段接入)

import { browser } from '$app/environment';

// 读 localStorage, 失败返 null
export function readStorage<T>(key: string): T | null {
	if (!browser) return null;
	try {
		const v = localStorage.getItem(key);
		return v === null ? null : (JSON.parse(v) as T);
	} catch (e) {
		console.warn(`[persist] read ${key} failed:`, e);
		return null;
	}
}

// 写 localStorage, 失败 swallow
export function writeStorage<T>(key: string, value: T) {
	if (!browser) return;
	try {
		localStorage.setItem(key, JSON.stringify(value));
	} catch (e) {
		console.warn(`[persist] write ${key} failed:`, e);
	}
}

// 删 localStorage
export function clearStorage(key: string) {
	if (!browser) return;
	localStorage.removeItem(key);
}

// ─── Mock 阶段: 不在 store 初始化时挂 $effect (Svelte 5 不允许模块级 $effect)
// 真实化阶段: 在 +page.svelte 或 +layout.svelte 里挂 effect, 调 writeStorage
// 或把整个 store 包成 factory, 在 component 中创建
