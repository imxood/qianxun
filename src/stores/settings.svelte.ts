import { call } from '../lib/ipc';
import type { Settings, SettingsPatch } from '../lib/ipc/contract';

/**
 * 设置域状态。合并逻辑只在 Rust 侧做一份：update 把补丁发过去，
 * 用返回的完整设置替换本地状态——避免前后端两套深合并互相漂移。
 */
class SettingsStore {
  current: Settings | null = $state(null);
  loadError: string | null = $state(null);

  async load(): Promise<void> {
    try {
      this.current = await call<Settings>('settings_get');
      this.loadError = null;
    } catch (error) {
      this.loadError = error instanceof Error ? error.message : String(error);
    }
  }

  async update(patch: SettingsPatch): Promise<void> {
    this.current = await call<Settings>('settings_update', { patch });
  }
}

export const settings = new SettingsStore();
