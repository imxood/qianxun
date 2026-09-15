import { getCurrentWindow } from '@tauri-apps/api/window';

/** 独立窗口支持的视图（与 Rust 侧 standalone_view_meta 对应）。 */
export type StandaloneView = 'dsh';

/** 当前 webview 的窗口 label（'main' 或 'standalone-{view}-{n}'）。 */
export const WINDOW_LABEL = getCurrentWindow().label;

/**
 * 解析当前页面是否为独立窗口视图。URL 形如
 * `index.html#/standalone/dsh`（Rust 侧 window_spawn_view 拼装）。
 */
export function standaloneView(): StandaloneView | null {
  const match = /#\/standalone\/(dsh)\b/.exec(window.location.hash);
  return match ? (match[1] as StandaloneView) : null;
}
