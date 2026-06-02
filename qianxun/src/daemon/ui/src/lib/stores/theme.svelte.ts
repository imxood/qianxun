// Stage 7a 主题 — light/dark/system 持久化到 localStorage.
// 集成 mode-watcher (从 qianxun-desktop 复用), Stage 7b 暴露 UI.

import { browser } from '$app/environment';
import { setMode, resetMode } from 'mode-watcher';

export type ThemeMode = 'light' | 'dark' | 'system';

const THEME_KEY = 'qianxun_web_theme';

class ThemeStore {
	#mode = $state<ThemeMode>('system');
	#initialized = $state(false);

	get mode(): ThemeMode {
		return this.#mode;
	}

	get initialized(): boolean {
		return this.#initialized;
	}

	init(): void {
		if (this.#initialized) return;
		if (!browser) return;
		try {
			const v = localStorage.getItem(THEME_KEY);
			if (v === 'light' || v === 'dark' || v === 'system') {
				this.#mode = v;
			}
		} catch {
			/* ignore */
		}
		// 把 mode 同步给 mode-watcher, 触发 <html class="dark"> 切换
		this.#apply();
		this.#initialized = true;
	}

	setMode(mode: ThemeMode): void {
		this.#mode = mode;
		if (browser) {
			try {
				localStorage.setItem(THEME_KEY, mode);
			} catch {
				/* ignore */
			}
		}
		this.#apply();
	}

	#apply(): void {
		if (this.#mode === 'system') {
			resetMode();
		} else {
			setMode(this.#mode);
		}
	}
}

export const themeStore = new ThemeStore();
