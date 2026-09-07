import { listen } from '@tauri-apps/api/event';
import { call } from '../lib/ipc';
import type { ThemePreference } from '../lib/ipc/contract';
import { resolveTheme } from '../lib/utils/theme';

/**
 * 主题域状态。偏好持久化在 settings.json（经 settings store），
 * 这里只负责「当前偏好 + 系统偏好 → 生效主题」的推导与监听；
 * DOM 类切换由 App.svelte / StandaloneWindow 的 $effect 完成。
 *
 * 系统偏好的权威来源是 Rust（注册表 AppsUseLightTheme + ThemeChanged
 * 推送）：WebView2 的 prefers-color-scheme 媒体查询默认恒报 light，
 * 「跟随系统」会失灵。matchMedia 只作 IPC 未就绪前的初始兜底。
 */
class ThemeStore {
  preference: ThemePreference = $state('system');
  systemDark: boolean = $state(false);
  resolved = $derived(resolveTheme(this.preference, this.systemDark));

  constructor() {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    this.systemDark = query.matches;
    // 媒体查询变化仅作兜底（部分 runtime 下 SetPreferredColorScheme
    // 会驱动它）；权威切换走 system://theme 推送。
    query.addEventListener('change', (event) => {
      this.systemDark = event.matches;
    });
    // 启动 seed：以 Rust 注册表读值为准，覆盖媒体查询的不可靠初值。
    void call<boolean>('system_theme')
      .then((dark) => {
        this.systemDark = dark;
      })
      .catch(() => {});
    // OS 深浅色实时切换（Rust ThemeChanged 钩子推送）。
    void listen<boolean>('system://theme', (event) => {
      this.systemDark = event.payload;
    }).catch(() => {});
  }

  set(preference: ThemePreference): void {
    this.preference = preference;
  }
}

export const theme = new ThemeStore();
