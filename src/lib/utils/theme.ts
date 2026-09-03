import type { ThemePreference } from '../ipc/contract';

/** 主题偏好的解析是纯函数，保证 store 只做状态、规则可测（编码规范 §9）。 */
export type ResolvedTheme = 'light' | 'dark';

export function resolveTheme(preference: ThemePreference, systemDark: boolean): ResolvedTheme {
  if (preference === 'system') return systemDark ? 'dark' : 'light';
  return preference;
}
