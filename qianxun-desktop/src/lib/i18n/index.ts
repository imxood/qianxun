// ───────────────────────────────────────────────────────────────────────────
// 千寻 Tauri 桌面端 — i18n (Stage 4 起步, 10 个 key)
// 与 docs/30_子项目规划/03-tauri-desktop.md §4.6 一致 (简化为自实现 60 行,
// 不引入 svelte-i18n 依赖, 后续 Stage 5 切换到 svelte-i18n 不用改业务调用).
//
// 范围: zh-CN + en 两个 locale, 10 个 key. Stage 5 扩到 30+ key + 日/法.
//
// 用法:
//   import { t, currentLocale, setLocale } from '$lib/i18n';
//   t('app.title')                              // '千寻' / 'Qianxun'
//   currentLocale.subscribe(v => console.log(v)) // 'zh-CN' | 'en'
//   setLocale('en')
// ───────────────────────────────────────────────────────────────────────────

import { writable, type Writable } from "svelte/store";
import zhCN from "./zh-CN";
import en from "./en";

export type Locale = "zh-CN" | "en";

/// 全部 key 的联合, t() 入参类型受限.
export type I18nKey = keyof typeof zhCN;

const messages: Record<Locale, Record<string, string>> = {
	"zh-CN": zhCN,
	en,
};

const STORAGE_KEY = "qianxun.locale";
const DEFAULT_LOCALE: Locale = "zh-CN";

/// 当前 locale (响应式). 持久化到 localStorage, 启动时回填.
function loadInitial(): Locale {
	if (typeof localStorage === "undefined") return DEFAULT_LOCALE;
	const raw = localStorage.getItem(STORAGE_KEY);
	if (raw === "zh-CN" || raw === "en") return raw;
	return DEFAULT_LOCALE;
}

export const currentLocale: Writable<Locale> = writable<Locale>(loadInitial());

/// 切换 locale 并持久化. SSR 阶段静默 no-op.
export function setLocale(loc: Locale): void {
	currentLocale.set(loc);
	try {
		localStorage.setItem(STORAGE_KEY, loc);
	} catch {
		// ignore (e.g. private mode)
	}
}

/// 翻译. 无 key 时返回 key 本身 (开发期可见, 不会崩 UI).
export function t(key: I18nKey): string {
	let loc: Locale = DEFAULT_LOCALE;
	currentLocale.subscribe((v) => (loc = v))(); // 立即同步取值
	return messages[loc][key] ?? messages["zh-CN"][key] ?? (key as string);
}
