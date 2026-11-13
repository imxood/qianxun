import { PhysicalPosition, PhysicalSize } from '@tauri-apps/api/dpi';
import type { Window } from '@tauri-apps/api/window';
import { call } from '../ipc';
import type { AppMetaResult, WorkArea } from '../ipc/contract';

/**
 * Linux（CEF 内嵌 Chromium）下窗口交互的补丁层。
 *
 * 背景（远程 omarchy 上实测）：
 * - CEF 把浏览器窗口 reparent 进 GTK content 区的 X11 子窗口，鼠标事件全落在
 *   子窗口上，GTK toplevel 收不到 pointer press → Tauri 内置的
 *   `start_dragging` 在 winit-gtk4 里拿不到指针状态，拖拽必然失败。
 * - GTK4 在 X11 下走客户端装饰（CSD），窗口管理器不会替 GTK4 窗口画边框，
 *   所以「让 WM 接管标题栏」这条路走不通，标题栏必须自绘。
 * - GTK4 的 `gtk_window_minimize()` 在 XWayland 下不产生 WM_STATE=Iconic，
 *   最小化按钮改为「隐藏到托盘」，语义上等价于收起窗口。
 * - XWayland 对没有 WM frame 的窗口（CSD 窗口正是如此）会忽略最大化请求：
 *   `toggle_maximize` 返回成功、`is_maximized` 也变成 true，但 Hyprland 看到的
 *   窗口尺寸纹丝不动（实测 1100x734 → 1100x734）。因此最大化也由前端自己做：
 *   记住原位置/尺寸 → 铺满**工作区** → 再切回来时还原。
 * - 铺满的目标矩形不能用 `screen.avail*`：Chromium 报的是整块屏幕，
 *   不知道 WM 的保留区（omarchy 左侧 28px 竖条），铺满会盖住它。
 *   工作区由 Rust 侧 `window_work_area` 向 WM 问（见
 *   `src-tauri/src/window/work_area.rs`），问不到才退回整屏。
 */

/** 运行平台是否为 Linux（取不到平台信息时按非 Linux 处理，走 Tauri 内置行为）。 */
export async function isLinuxRuntime(): Promise<boolean> {
  try {
    const meta = await call<AppMetaResult>('app_meta');
    return meta.platform === 'linux';
  } catch {
    return false;
  }
}

/**
 * 把一个元素变成 Linux 下的窗口拖拽把手。
 *
 * 做法：按下时记下指针与窗口位置，移动时按位移量调用 `set_position`。
 * 指针捕获（setPointerCapture）保证指针移出窗口后仍能收到 move 事件。
 * 返回解绑函数。
 */
export function attachLinuxDrag(
  el: HTMLElement,
  win: Window,
  options: { onDoubleClick?: () => void } = {},
): () => void {
  let dragging = false;
  let ready = false;
  let startX = 0;
  let startY = 0;
  let originX = 0;
  let originY = 0;
  let scale = 1;
  let lastX = 0;
  let lastY = 0;
  let lastClickAt = 0;
  let lastClickX = 0;
  let lastClickY = 0;

  // 直接调 set_position，不用 requestAnimationFrame：
  // 窗口被遮挡/未聚焦时 Chromium 会挂起 rAF，拖拽会整段失效。
  const apply = () => {
    if (!ready) return;
    const x = originX + (lastX - startX) * scale;
    const y = originY + (lastY - startY) * scale;
    void win.setPosition(new PhysicalPosition(x, y)).catch(() => {});
  };

  const onPointerDown = (event: PointerEvent) => {
    if (event.button !== 0) return;
    // 按钮/链接等可点元素不触发拖拽（与 Tauri 内置脚本的行为一致）。
    const target = event.target as HTMLElement | null;
    if (target?.closest('button, a, input, select, textarea, [role="button"]')) return;
    event.preventDefault();
    try {
      el.setPointerCapture(event.pointerId);
    } catch {
      /* 某些环境不允许捕获，退化成普通事件流 */
    }
    dragging = true;
    ready = false;
    startX = event.screenX;
    startY = event.screenY;
    lastX = startX;
    lastY = startY;

    // 双击判定：两次按下间隔很短且位置接近 → 视为双击（标题栏双击最大化）。
    const now = event.timeStamp || Date.now();
    const isDouble =
      now - lastClickAt < 400 &&
      Math.abs(event.screenX - lastClickX) < 6 &&
      Math.abs(event.screenY - lastClickY) < 6;
    lastClickAt = now;
    lastClickX = event.screenX;
    lastClickY = event.screenY;
    if (isDouble) {
      dragging = false;
      ready = false;
      options.onDoubleClick?.();
      return;
    }

    void (async () => {
      try {
        const [pos, factor] = await Promise.all([win.outerPosition(), win.scaleFactor()]);
        originX = pos.x;
        originY = pos.y;
        scale = factor;
        ready = true;
        // 位置是异步拿到的，这里补一次，保证按下后立刻移动也能生效。
        apply();
      } catch {
        ready = false;
      }
    })();
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!dragging) return;
    lastX = event.screenX;
    lastY = event.screenY;
    apply();
  };

  const onPointerUp = (event: PointerEvent) => {
    if (!dragging) return;
    dragging = false;
    ready = false;
    try {
      el.releasePointerCapture(event.pointerId);
    } catch {
      /* 已释放 */
    }
  };

  el.addEventListener('pointerdown', onPointerDown);
  el.addEventListener('pointermove', onPointerMove);
  el.addEventListener('pointerup', onPointerUp);
  el.addEventListener('pointercancel', onPointerUp);

  return () => {
    el.removeEventListener('pointerdown', onPointerDown);
    el.removeEventListener('pointermove', onPointerMove);
    el.removeEventListener('pointerup', onPointerUp);
    el.removeEventListener('pointercancel', onPointerUp);
  };
}

/** Linux 下的最大化控制器：自己算几何，不依赖窗口管理器。 */
export interface LinuxMaximize {
  /** 当前是否处于「自绘最大化」状态。 */
  isMaximized(): boolean;
  /** 最大化 / 还原。 */
  toggle(): Promise<void>;
}

/**
 * 取当前窗口所在显示器的可用工作区（物理像素）。
 *
 * `screen.avail*` 与 Tauri 的 `monitor.work_area()` 都只认整块屏幕——不知道
 * 窗口管理器的保留区（omarchy 左侧 28px 竖条，Hyprland 的 reserved）。
 * 铺满整屏会把那根竖条盖住，所以优先问 Rust 侧（它直接问 WM）。
 * 问不到就退回整屏（旧行为）。
 */
async function resolveWorkArea(): Promise<WorkArea | null> {
  try {
    const area = await call<WorkArea>('window_work_area');
    if (!area || area.width <= 0 || area.height <= 0) return null;
    return area;
  } catch {
    return null;
  }
}

export function createLinuxMaximize(win: Window): LinuxMaximize {
  let maximized = false;
  let saved: { x: number; y: number; width: number; height: number } | null = null;

  const toggle = async () => {
    if (!maximized) {
      const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
      saved = { x: pos.x, y: pos.y, width: size.width, height: size.height };
      const area = await resolveWorkArea();
      if (area) {
        // 工作区已是物理像素，直接用。
        await win.setSize(new PhysicalSize(area.width, area.height));
        await win.setPosition(new PhysicalPosition(area.x, area.y));
      } else {
        const factor = await win.scaleFactor();
        const screen = window.screen as Screen & { availLeft?: number; availTop?: number };
        const cssWidth = screen.availWidth || screen.width;
        const cssHeight = screen.availHeight || screen.height;
        const cssLeft = screen.availLeft || 0;
        const cssTop = screen.availTop || 0;
        await win.setSize(
          new PhysicalSize(Math.round(cssWidth * factor), Math.round(cssHeight * factor)),
        );
        await win.setPosition(
          new PhysicalPosition(Math.round(cssLeft * factor), Math.round(cssTop * factor)),
        );
      }
      maximized = true;
      return;
    }
    if (saved) {
      await win.setSize(new PhysicalSize(saved.width, saved.height));
      await win.setPosition(new PhysicalPosition(saved.x, saved.y));
      saved = null;
    }
    maximized = false;
  };

  return {
    isMaximized: () => maximized,
    toggle,
  };
}
