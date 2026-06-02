// Stage 7b — i18n store (minimal in-house, mirrors qianxun-desktop pattern)
//
// 选用原因:
//   1. svelte-i18n 不在 Stage 7a devDeps 里, 装需要追加 pnpm install (~200KB extra)
//   2. 50 个 key 的小规模翻译, 不需要 ICU plural / interpolation 引擎
//   3. 跟 qianxun-desktop 用同样模式 (writable + t() helper), 团队一致性
//
// 用法:
//   import { t, locale, setLocale } from '$lib/i18n';
//   t('panel.llm.title')                        // 'LLM Providers' / 'LLM 接入'
//   locale.subscribe(v => console.log(v))       // 'zh-CN' | 'en'
//   setLocale('en')
//
// 范围: 60 个 key (4 旧面板 + 4 新面板 + 通用 UI), 后续 Stage 7c/8 扩展.

import { writable, type Writable } from 'svelte/store';
import zhCN from './zh-CN.json';
import en from './en.json';

export type Locale = 'zh-CN' | 'en';

const STORAGE_KEY = 'qianxun_lang';
const DEFAULT_LOCALE: Locale = 'zh-CN';

const messages: Record<Locale, Record<string, string>> = {
	'zh-CN': zhCN,
	en
};

/// 全部 key 联合类型 (I18nKey) — 自动从 zh-CN 推导.
export type I18nKey = keyof typeof zhCN;

function loadInitial(): Locale {
	if (typeof localStorage === 'undefined') return DEFAULT_LOCALE;
	try {
		const v = localStorage.getItem(STORAGE_KEY);
		if (v === 'zh-CN' || v === 'en') return v;
	} catch {
		/* ignore */
	}
	return DEFAULT_LOCALE;
}

export const locale: Writable<Locale> = writable<Locale>(loadInitial());

export function setLocale(l: Locale): void {
	locale.set(l);
	try {
		if (typeof localStorage !== 'undefined') {
			localStorage.setItem(STORAGE_KEY, l);
		}
	} catch {
		/* ignore */
	}
}

/// 当前 locale 一次性取值 (非响应式, 给非组件代码用)
export function currentLocale(): Locale {
	let v: Locale = DEFAULT_LOCALE;
	locale.subscribe((x) => (v = x))();
	return v;
}

/**
 * 翻译. 无 key 时返回 key 本身 (开发期可见, 不会崩 UI).
 * 找不到当前 locale 的 key 时 fallback 到 zh-CN, 再找不到就返 key 字符串.
 */
export function t(key: I18nKey | string): string {
	const loc = currentLocale();
	const msg = messages[loc]?.[key] ?? messages['zh-CN']?.[key] ?? (key as string);
	return msg;
}

/**
 * 响应式版本的 t. 在 Svelte 5 组件里用 `let tt = $derived(t('key'))` 包一下即可.
 * 或用底下的 useT() 在 .svelte 模板里直接 `{$t('key')}` (注意: 没有 $t, 用 tt 模式).
 */
export function useT() {
	return {
		t: (k: I18nKey | string) => t(k)
	};
}

/// 测试 + 程序化用: 给定 locale 查 raw message
export function rawMessage(loc: Locale, key: string): string | undefined {
	return messages[loc]?.[key];
}

/// 全部 locales (给 Settings 等面板遍历用)
export const ALL_LOCALES: { value: Locale; label: string }[] = [
	{ value: 'zh-CN', label: '简体中文' },
	{ value: 'en', label: 'English' }
];
