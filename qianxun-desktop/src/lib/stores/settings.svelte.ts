// ───────────────────────────────────────────────────────────────────────────
// SettingsStore — Stage 5 §11 主题/缓存/Settings 配置
// 与 docs/30_子项目规划/03-tauri-desktop.md §11.1 Settings 模型一致
//
// 5 字段 (2026-06-09 加 activeProvider):
//   - theme:    'light' | 'dark' | 'system' (用户手动选, 不强制监听 OS, 让用户决定)
//   - locale:   'zh-CN' | 'en'
//   - daemonUrl: 默认 'http://127.0.0.1:23900'
//   - vpsUrl:   默认 '' (未配置, 见 VpsStore)
//   - activeProvider: 当前激活 provider 名 ('deepseek' / 'MiniMax' / 自定义), 调 invoke
//                    写 ~/.qianxun/config.json, **需重启 desktop 生效**
//
// 持久化: localStorage (key = 'qianxun-settings'), Svelte 5 $effect.root 自动同步.
// 启动时回填: 立即在 constructor 阶段读 storage (SSR 静默返回默认值).
//
// Stage 6a: stronghold 凭据加密函数已就绪 (见 ipc/bridge.ts setSecret/getSecret).
// Stage 6b: vpsToken 仍以明文形式存于独立 localStorage key 'qianxun.vps.token'
//   (脱敏存储, 不进 settings 持久化), Settings 页不暴露, 暂仅供 store 内部
//   真实 fetch 时取用. Stage 7 加密码弹窗 + stronghold.
// ───────────────────────────────────────────────────────────────────────────

import { updateActiveProvider } from "$lib/ipc/runtime";
import { uiStore } from "./ui.svelte";

export type Theme = "light" | "dark" | "system";
export type Locale = "zh-CN" | "en";

const STORAGE_KEY = "qianxun-settings";
const VPS_TOKEN_STORAGE_KEY = "qianxun.vps.token"; // Stage 6b: 独立 key, 未来 stronghold 替换
const DEFAULT_THEME: Theme = "system";
const DEFAULT_LOCALE: Locale = "zh-CN";
const DEFAULT_DAEMON_URL = "http://127.0.0.1:23900";
const DEFAULT_ACTIVE_PROVIDER = "deepseek";

interface PersistedSettings {
	theme: Theme;
	locale: Locale;
	daemonUrl: string;
	vpsUrl: string;
	activeProvider: string;
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
			activeProvider:
				typeof parsed.activeProvider === "string" && parsed.activeProvider
					? parsed.activeProvider
					: DEFAULT_ACTIVE_PROVIDER,
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
		activeProvider: DEFAULT_ACTIVE_PROVIDER,
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
	/// 2026-06-09 加: 当前激活 provider 名. 桌面端设置 UI 改这个会调 invoke
	/// `update_active_provider` 写 ~/.qianxun/config.json, **不热替换**, 需重启.
	activeProvider = $state<string>(DEFAULT_ACTIVE_PROVIDER);

	/// Stage 6b: VPS access token. 暂不暴露 UI, 真实 fetch 时由 vpsStore 内部
	/// 通过 getVpsToken() 取. Stage 7 替换为 stronghold 加密 + 密码弹窗.
	vpsToken = $state<string>("");

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
		activeProvider: this.activeProvider,
	}));

	constructor() {
		const init = loadInitial();
		this.theme = init.theme;
		this.locale = init.locale;
		this.daemonUrl = init.daemonUrl;
		this.vpsUrl = init.vpsUrl;
		this.activeProvider = init.activeProvider;

		// Stage 6b: 启动时从独立 localStorage key 读 vpsToken (明文 fallback).
		// Stage 7 替换为: getSecret('vps-token', userInputPassword).
		try {
			if (typeof localStorage !== "undefined") {
				const t = localStorage.getItem(VPS_TOKEN_STORAGE_KEY);
				if (t) this.vpsToken = t;
			}
		} catch {
			// ignore
		}

		// 自动持久化: $effect.root 创建非追踪的 effect, 5 字段任意一个变就回写 storage.
		// jsdom 环境下 localStorage 存在, 真实 SSR 下 effect 也会跑但写不到 storage (catch).
		$effect.root(() => {
			$effect(() => {
				const snapshot: PersistedSettings = {
					theme: this.theme,
					locale: this.locale,
					daemonUrl: this.daemonUrl,
					vpsUrl: this.vpsUrl,
					activeProvider: this.activeProvider,
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

	/// 2026-06-09 加: 设置 active provider. 调 invoke 写 ~/.qianxun/config.json.
	/// 成功提示用户重启 desktop. 失败弹 toast 错误.
	async setActiveProvider(provider: string, config?: {
		api_key?: string;
		model?: string;
		base_url?: string;
		temperature?: number;
		max_tokens?: number;
	}): Promise<void> {
		const name = provider.trim();
		if (!name) return;
		this.activeProvider = name; // 本地立即更新 (UI 显示)
		try {
			await updateActiveProvider({
				active_provider: name,
				provider_config: config,
			});
			uiStore.pushToast({
				kind: "success",
				title: "Provider 已更新",
				body: `${name} 配置已保存, 需重启 desktop 生效`,
				timeout_ms: 5000,
			});
		} catch (e) {
			const msg = (e as Error).message ?? String(e);
			uiStore.pushToast({
				kind: "error",
				title: "Provider 更新失败",
				body: msg,
				timeout_ms: 5000,
			});
		}
	}

	/// Stage 6b: 存 vps token 到 localStorage (明文). Stage 7 替换为 setSecret().
	setVpsToken(token: string): void {
		this.vpsToken = token;
		try {
			if (typeof localStorage !== "undefined") {
				if (token) {
					localStorage.setItem(VPS_TOKEN_STORAGE_KEY, token);
				} else {
					localStorage.removeItem(VPS_TOKEN_STORAGE_KEY);
				}
			}
		} catch {
			// ignore
		}
	}

	/// Stage 6b: 取 vps token (vpsStore 真实 fetch 时调). Stage 7 替换为
	/// getSecret('vps-token', pwd) — 需用户先输密码.
	getVpsToken(): string {
		return this.vpsToken;
	}

	/// 重置全部字段到默认值 (Stage 6 Settings 页 "恢复默认" 按钮会用到).
	reset(): void {
		this.theme = DEFAULT_THEME;
		this.locale = DEFAULT_LOCALE;
		this.daemonUrl = DEFAULT_DAEMON_URL;
		this.vpsUrl = "";
		this.activeProvider = DEFAULT_ACTIVE_PROVIDER;
		this.setVpsToken(""); // 同时清掉 token
	}
}

export const settingsStore = new SettingsStore();
