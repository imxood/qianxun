import type { ThemePreference } from '../lib/ipc/contract';
import { resolveTheme } from '../lib/utils/theme';

/**
 * 主题域状态。偏好持久化在 settings.json（经 settings store），
 * 这里只负责「当前偏好 + 系统偏好 → 生效主题」的推导与监听；
 * DOM 类切换由 App.svelte 的 $effect 完成（组件上下文才能开 effect）。
 */
class ThemeStore {
  preference: ThemePreference = $state('system');
  systemDark: boolean = $state(false);
  resolved = $derived(resolveTheme(this.preference, this.systemDark));

  constructor() {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    this.systemDark = query.matches;
    // 系统偏好变化只改状态，不直接碰 DOM——单一数据流方向。
    query.addEventListener('change', (event) => {
      this.systemDark = event.matches;
    });
  }

  set(preference: ThemePreference): void {
    this.preference = preference;
  }
}

export const theme = new ThemeStore();
