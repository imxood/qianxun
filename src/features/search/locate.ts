import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener';
import { search } from '../../stores/search.svelte';
import { toast } from '../../stores/toast.svelte';

/** 相对路径 → 绝对路径（根目录 + \ 分隔）。 */
export function absolutePath(relative: string): string | null {
  const root = search.status?.root;
  if (!root) return null;
  return `${root}\\${relative.replace(/\//g, '\\')}`;
}

/** 用系统默认程序打开文件。失败弹 toast（汇总 §3.16）：之前 `.catch(() => {})`
 * 静默吞错违反 docs/03 §8，文件被外部删除 / 权限拒绝时用户零反馈。 */
export function openFile(relative: string): void {
  const target = absolutePath(relative);
  if (!target) {
    toast.show('未选择搜索根目录');
    return;
  }
  void openPath(target).catch((error: unknown) => {
    toast.show(`打开失败：${relative}`, {
      label: '重试',
      run: () => openFile(relative),
    });
    if (import.meta.env.DEV) {
      console.error('[locate.openFile] failed', target, error);
    }
  });
}

/** 在资源管理器中定位文件。失败弹 toast。 */
export function locateInExplorer(relative: string): void {
  const target = absolutePath(relative);
  if (!target) {
    toast.show('未选择搜索根目录');
    return;
  }
  void revealItemInDir(target).catch((error: unknown) => {
    toast.show(`定位失败：${relative}`, {
      label: '重试',
      run: () => locateInExplorer(relative),
    });
    if (import.meta.env.DEV) {
      console.error('[locate.locateInExplorer] failed', target, error);
    }
  });
}

/** 写剪贴板（右键菜单点击算用户手势，WebView2 放行）。返回是否成功。 */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch (error: unknown) {
    toast.show('复制失败：剪贴板权限被拒');
    if (import.meta.env.DEV) {
      console.error('[locate.copyText] failed', text, error);
    }
    return false;
  }
}

/** 结果行的复制菜单：三种路径形态 + 文件名（设计 §3.4）。 */
export function copyMenuItems(relative: string): Array<{
  label: string;
  onclick: () => void;
}> {
  const absolute = absolutePath(relative) ?? relative;
  const name = relative.split(/[\\/]/).pop() ?? relative;
  const quoted = `"${absolute}"`;
  const posix = absolute.replace(/\\/g, '/');
  const item = (label: string, value: string) => ({
    label,
    onclick: () => void copyText(value),
  });
  return [
    item('复制路径', absolute),
    item('复制路径（带引号）', quoted),
    item('复制路径（正斜杠）', posix),
    item('复制文件名', name),
  ];
}
