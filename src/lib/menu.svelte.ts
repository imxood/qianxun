/**
 * 全局右键菜单（单例 store + 挂载层组件）：任何页面用
 * `contextMenu.show(event, items)` 弹出，`<ContextMenuLayer />` 负责渲染。
 * 坐标在 show 时按视口收边，避免菜单出屏。
 */

export interface MenuItem {
  label: string;
  onclick?: () => void;
  /** 危险动作红色显示（如删除）。 */
  danger?: boolean;
}

class ContextMenuStore {
  x = $state(0);
  y = $state(0);
  items: MenuItem[] = $state([]);

  get visible(): boolean {
    return this.items.length > 0;
  }

  show(event: MouseEvent, items: MenuItem[]): void {
    if (items.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    // 收边：菜单约 220px 宽、每项 32px 高（含少量内边距余量）。
    this.x = Math.min(event.clientX, window.innerWidth - 232);
    this.y = Math.min(event.clientY, window.innerHeight - items.length * 32 - 20);
    this.items = items;
  }

  close(): void {
    this.items = [];
  }

  run(item: MenuItem): void {
    this.close();
    item.onclick?.();
  }
}

export const contextMenu = new ContextMenuStore();
