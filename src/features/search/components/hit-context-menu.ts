/**
 * 通用结果行右键菜单（汇总 §3.3 抽公共组件 · 第一阶段）。
 *
 * 之前 FilesPage / GrepPage / DiskScan 三处独立 import `locate.ts` 的
 * `copyMenuItems` 等辅助函数 + 自己拼菜单，导致右键菜单的 label 文案 / 分组
 * 顺序不一致。本模块统一渲染入口：
 *
 * - 单行模式：打开 / 在资源管理器中显示 / 复制路径（三种形态 + 文件名）
 * - 多行模式：在资源管理器中显示第一项 / 复制 N 条路径
 *
 * 错误反馈统一走 `toast` store（汇总 §3.16）。
 */
import { copyMenuItems, locateInExplorer, openFile } from '../locate';
import { toast } from '../../../stores/toast.svelte';
import { contextMenu, type MenuItem } from '../../../lib/menu.svelte';

/** 单行右键菜单项：打开 / 定位 / 复制路径（含三种形态 + 文件名）。 */
export function singleHitMenu(relative: string): MenuItem[] {
  const copies = copyMenuItems(relative);
  return [
    { label: '打开文件', onclick: () => openFile(relative) },
    { label: '在资源管理器中显示', onclick: () => locateInExplorer(relative) },
    ...copies.map((c) => ({ label: c.label, onclick: c.onclick })),
  ];
}

/** 多行右键菜单项：定位第一项 / 批量复制。 */
export function multiHitMenu(paths: string[]): MenuItem[] {
  const first = paths[0] ?? '';
  return [
    {
      label: `在资源管理器中显示 ${first.split(/[\\/]/).pop() ?? ''}`,
      onclick: () => locateInExplorer(first),
    },
    {
      label: `复制 ${paths.length} 条路径`,
      onclick: () => {
        const joined = paths.join('\n');
        void navigator.clipboard.writeText(joined).then(
          () => {
            /* 成功无需 toast */
          },
          () => {
            toast.show(`复制 ${paths.length} 条路径失败`);
          },
        );
      },
    },
  ];
}

/** 一站式入口：根据选中数量自动选单/多形态。 */
export function showHitContextMenu(
  event: MouseEvent,
  relative: string | undefined,
  paths: string[],
): void {
  const items = paths.length > 1 ? multiHitMenu(paths) : singleHitMenu(relative ?? '');
  contextMenu.show(event, items);
}
