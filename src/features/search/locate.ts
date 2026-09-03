import { revealItemInDir } from '@tauri-apps/plugin-opener';
import { search } from '../../stores/search.svelte';

/** 在资源管理器中定位相对路径（root + 相对路径拼绝对路径）。 */
export function locateInExplorer(relative: string): void {
  const root = search.status?.root;
  if (!root) return;
  const absolute = `${root}\\${relative.replace(/\//g, '\\')}`;
  void revealItemInDir(absolute).catch(() => {
    // 定位失败（路径消失等）静默：列表刷新后自然纠正。
  });
}
