import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener';
import { search } from '../../stores/search.svelte';

/** 相对路径 → 绝对路径（根目录 + \ 分隔）。 */
function absolute(relative: string): string | null {
  const root = search.status?.root;
  if (!root) return null;
  return `${root}\\${relative.replace(/\//g, '\\')}`;
}

/** 用系统默认程序打开文件。失败静默：文件可能已被移动或删除。 */
export function openFile(relative: string): void {
  const target = absolute(relative);
  if (!target) return;
  void openPath(target).catch(() => {});
}

/** 在资源管理器中定位文件。失败静默：列表刷新后自然纠正。 */
export function locateInExplorer(relative: string): void {
  const target = absolute(relative);
  if (!target) return;
  void revealItemInDir(target).catch(() => {});
}
