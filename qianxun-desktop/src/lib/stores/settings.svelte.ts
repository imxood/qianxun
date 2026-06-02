// ───────────────────────────────────────────────────────────────────────────
// SettingsStore — Stage 5 §11 主题/缓存/Settings 配置
// 与 docs/30_子项目规划/03-tauri-desktop.md §11.1 Settings 模型一致
//
// 4 字段:
//   - theme:    'light' | 'dark' | 'system' (用户手动选, 不强制监听 OS, 让用户决定)
//   - locale:   'zh-CN' | 'en'
//   - daemonUrl: 默认 'http://127.0.0.1:23900'
//   - vpsUrl:   默认 '' (未配置, 见 VpsStore)
//
// 持久化: localStorage (key = 'qianxun-settings'), Svelte 5 $effect.root 自动同步.
// 启动时回填: 立即在 constructor 阶段读 storage (SSR 静默返回默认值).
//
// Stage 6 TODO: 凭据加密 (Tauri stronghold) — 现在 vpsUrl 仍明文 localStorage.
// ───────────────────────────────────────────────────────────────────────────

export type Theme = "light" | "dark" | "system";
export type Locale = "zh-CN" | "en";

const STORAGE_KEY = "qianxun-settings";
const DEFAULT_THEME: Theme = "system";
const DEFAULT_LOCALE: Locale = "zh-CN";
const DEFAULT_DAEMON_URL = "http://127.0.0.1:23900";

interface PersistedSettings {
	theme: Theme;
	locale: Locale;
	daemonUrl: string;
	vpsUrl: string;
}

function loadInitial(): PersistedSettings {
	if (typeof localStorage === "undefined") {
		return defaults();
	}
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return defaults();
		const parsed = JSON.parse(raw) as Partial<PersistedSettings>;
		return {
			theme: isTheme(parsed.theme) ? parsed.theme : DEFAULT_THEME,
			locale: isLocale(parsed.locale) ? parsed.locale : DEFAULT_LOCALE,
			daemonUrl: typeof parsed.daemonUrl === "string" ? parsed.daemonUrl : DEFAULT_DAEMON_URL,
			vpsUrl: typeof parsed.vpsUrl === "string" ? parsed.vpsUrl : "",
		};
	} catch {
		return defaults();
	}
}

function defaults(): PersistedSettings {
	return {
		theme: DEFAULT_THEME,
		locale: DEFAULT_LOCALE,
		daemonUrl: DEFAULT_DAEMON_URL,
		vpsUrl: "",
	};
}

function isTheme(v: unknown): v is Theme {
	return v === "light" || v === "dark" || v === "system";
}

function isLocale(v: unknown): v is Locale {
	return v === "zh-CN" || v === "en";
}

class SettingsStore {
	theme = $state<Theme>(DEFAULT_THEME);
	locale = $state<Locale>(DEFAULT_LOCALE);
	daemonUrl = $state<string>(DEFAULT_DAEMON_URL);
	vpsUrl = $state<string>("");

	// ─── 派生 ────────────────────────────────────────────────────────────────

	/// 实际渲染用的主题: 'system' 时跟 window.matchMedia. (Stage 5 简化:
	/// 默认走 'light' 当 matchMedia 不可用, 真实 system 监听留 Stage 6.)
	resolvedTheme = $derived.by<"light" | "dark">(() => {
		if (this.theme === "light" || this.theme === "dark") return this.theme;
		if (typeof window === "undefined" || !window.matchMedia) return "light";
		return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
	});

	/// 序列化到 localStorage 的快照 (供外部导出/调试)
	serialized = $derived.by<PersistedSettings>(() => ({
		theme: this.theme,
		locale: this.locale,
		daemonUrl: this.daemonUrl,
		vpsUrl: this.vpsUrl,
	}));

	constructor() {
		const init = loadInitial();
		this.theme = init.theme;
		this.locale = init.locale;
		this.daemonUrl = init.daemonUrl;
		this.vpsUrl = init.vpsUrl;

		// 自动持久化: $effect.root 创建非追踪的 effect, 4 字段任意一个变就回写 storage.
		// jsdom 环境下 localStorage 存在, 真实 SSR 下 effect 也会跑但写不到 storage (catch).
		$effect.root(() => {
			$effect(() => {
				const snapshot: PersistedSettings = {
					theme: this.theme,
					locale: this.locale,
					daemonUrl: this.daemonUrl,
					vpsUrl: this.vpsUrl,
				};
				try {
					localStorage.setItem(STORAGE_KEY, JSON.stringify(snapshot));
				} catch {
					// ignore (private mode, SSR, etc.)
				}
			});
		});
	}

	// ─── 写入助手 ────────────────────────────────────────────────────────────

	setTheme(theme: Theme): void {
		this.theme = theme;
	}

	setLocale(locale: Locale): void {
		this.locale = locale;
	}

	setDaemonUrl(url: string): void {
		this.daemonUrl = url.trim();
	}

	setVpsUrl(url: string): void {
		this.vpsUrl = url.trim();
	}

	/// 重置全部字段到默认值 (Stage 6 Settings 页 "恢复默认" 按钮会用到).
	reset(): void {
		this.theme = DEFAULT_THEME;
		this.locale = DEFAULT_LOCALE;
		this.daemonUrl = DEFAULT_DAEMON_URL;
		this.vpsUrl = "";
	}
}

export const settingsStore = new SettingsStore();
