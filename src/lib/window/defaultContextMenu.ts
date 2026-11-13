/**
 * 默认右键菜单（应用层兜底，压掉 CEF 原生菜单弹窗）。
 *
 * 背景（Linux/Arch + tauri v3 `tauri-runtime-cef` 实测）：
 * CEF 的原生右键菜单是**浏览器进程里 views 自绘的 popup**，在 X11 下是一个独立的
 * override-redirect 顶层窗口。在「GTK4 toplevel 套 CEF 子窗口」的嵌套结构里，这个
 * popup 会被算成与主窗口等大（实测 1100x734，位置比主窗口偏 24px，正好是
 * `_GTK_FRAME_EXTENTS` 的偏移），于是用户看到的是一块巨大的空白面板挡住界面；
 * 而且该窗口可能被窗口管理器抢焦点后立刻消失（CEF issue #4329 / PR #4346）。
 *
 * 有效修法（已实测）：在渲染进程里对 `contextmenu` 事件调 `preventDefault()`，
 * CEF 就不再创建那个原生弹窗。千寻自己的右键菜单（`src/lib/menu.svelte.ts` 的
 * `contextMenu` store + `<ContextMenuLayer />`）本来就是 DOM 实现的，所以这条
 * 修复同时得到「没有巨大弹窗」和「有可用菜单」两个结果。
 *
 * 模块分工：
 * - `describeContextMenuContext()`：从事件目标算出上下文（可编辑？有没有选区？），
 *   这一步要碰 DOM。
 * - `buildDefaultMenuItems()`：纯函数，只按上下文决定菜单项与禁用态，便于单测。
 */

import type { MenuItem } from '../menu.svelte';

/** 右键上下文：够用即可，避免把整个 DOM 塞进菜单项闭包。 */
export interface DefaultMenuContext {
  /** 命中的目标是否可编辑（文本类 input / textarea / contenteditable）。 */
  editable: boolean;
  /** 当前选中的文本（可编辑时取输入框内选区，否则取页面选区）。 */
  selection: string;
  /** 是否存在非空选区（纯空白不算）。 */
  hasSelection: boolean;
  /** 命中的可编辑元素（可编辑时非空），用于剪切/粘贴/全选定位。 */
  element?: HTMLElement | null;
}

/** 可编辑的 input 类型（其余类型如 checkbox/radio/button 不可编辑文本）。 */
const EDITABLE_INPUT_TYPES = new Set(['text', 'search', 'url', 'email', 'password']);

/**
 * 找到右键命中的可编辑元素。
 *
 * 用 `closest()` 是因为 contenteditable 内部往往还有一层 span/div 承载文字，
 * 事件目标未必就是可编辑宿主本身。`isContentEditable` 能正确识别
 * `contenteditable="false"` 的嵌套子块（那种是不可编辑的）。
 */
function findEditableElement(target: EventTarget | null): HTMLElement | null {
  if (!(target instanceof Element)) return null;
  const candidate = target.closest('input, textarea, [contenteditable]');
  if (!candidate) return null;
  if (candidate instanceof HTMLInputElement) {
    return EDITABLE_INPUT_TYPES.has(candidate.type.toLowerCase()) ? candidate : null;
  }
  if (candidate instanceof HTMLTextAreaElement) return candidate;
  if (candidate instanceof HTMLElement && candidate.isContentEditable) return candidate;
  return null;
}

/** 取选区文本：可编辑元素取框内选区，普通页面取窗口选区。 */
function readSelection(element: HTMLElement | null): string {
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    const start = element.selectionStart ?? 0;
    const end = element.selectionEnd ?? 0;
    return end > start ? element.value.slice(start, end) : '';
  }
  return window.getSelection()?.toString() ?? '';
}

/** 从右键事件目标算出菜单上下文（DOM 相关，不能进纯函数单测）。 */
export function describeContextMenuContext(target: EventTarget | null): DefaultMenuContext {
  const element = findEditableElement(target);
  const selection = readSelection(element);
  return {
    editable: element !== null,
    selection,
    // 纯空白选区视为「没选中」：否则会弹出一个复制空串的菜单。
    hasSelection: selection.trim().length > 0,
    element,
  };
}

/** 复制选区到剪贴板。失败静默（用户手势内浏览器可能拒权，不值得弹 toast）。 */
async function copySelection(selection: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(selection);
  } catch {
    /* 剪贴板权限被拒：忽略 */
  }
}

/** 剪切：优先用 execCommand（能同步更新 input 绑定），失败退化为写剪贴板 + 删选区。 */
function cutSelection(ctx: DefaultMenuContext): void {
  const element = ctx.element;
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    try {
      if (document.execCommand('cut')) return;
    } catch {
      /* 落到退化分支 */
    }
  }
  void copySelection(ctx.selection);
  deleteSelection(element ?? null);
}

/** 删除当前选区（退化剪切路径用）。 */
function deleteSelection(element: HTMLElement | null): void {
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    const start = element.selectionStart ?? 0;
    const end = element.selectionEnd ?? start;
    element.setRangeText('', start, end, 'end');
    // 手动派发 input，让 Svelte 的 value 绑定跟上。
    element.dispatchEvent(new Event('input', { bubbles: true }));
    return;
  }
  try {
    document.execCommand('delete');
  } catch {
    /* 忽略 */
  }
}

/** 读剪贴板文本；被拒权时返回 null（区分「空文本」与「读不到」）。 */
async function readClipboardText(): Promise<string | null> {
  try {
    return await navigator.clipboard.readText();
  } catch {
    return null;
  }
}

/** 粘贴：读剪贴板文本后插入到当前焦点/选区位置。 */
async function pasteInto(ctx: DefaultMenuContext): Promise<void> {
  const text = await readClipboardText();
  if (text === null) return;
  const element = ctx.element;
  try {
    if (document.execCommand('insertText', false, text)) return;
  } catch {
    /* 落到 setRangeText 分支 */
  }
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    const start = element.selectionStart ?? element.value.length;
    const end = element.selectionEnd ?? start;
    element.setRangeText(text, start, end, 'end');
    element.dispatchEvent(new Event('input', { bubbles: true }));
  }
}

/** 全选：可编辑元素全选其内容，否则全选页面文本。 */
function selectAll(ctx: DefaultMenuContext): void {
  const element = ctx.element;
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    element.select();
    element.focus();
    return;
  }
  if (element) {
    // contenteditable 宿主自带 select()（TS 的 lib.dom 只在 input/textarea 上声明了它）。
    const selectable = element as HTMLElement & { select?: () => void };
    selectable.select?.();
    return;
  }
  window.getSelection()?.selectAllChildren(document.body);
}

/**
 * 按上下文构造默认菜单项（纯函数，不碰 DOM，便于单测）。
 *
 * 规则：
 * - 可编辑 → 剪切 / 复制 / 粘贴 / 全选；剪切与复制在没有选区时禁用。
 * - 不可编辑但有选中文本 → 复制 / 全选（全选在此走 `selectAllChildren` 整页选）。
 * - 两者都没有 → 空数组（调用方仍然 preventDefault，只是不弹菜单，避免空面板）。
 */
export function buildDefaultMenuItems(ctx: DefaultMenuContext): MenuItem[] {
  if (ctx.editable) {
    return [
      {
        label: '剪切',
        disabled: !ctx.hasSelection,
        onclick: () => cutSelection(ctx),
      },
      {
        label: '复制',
        disabled: !ctx.hasSelection,
        onclick: () => void copySelection(ctx.selection),
      },
      {
        label: '粘贴',
        // 非可编辑时粘贴无意义（规则要求禁用）；可编辑分支里恒为 false。
        disabled: !ctx.editable,
        onclick: () => void pasteInto(ctx),
      },
      {
        label: '全选',
        onclick: () => selectAll(ctx),
      },
    ];
  }
  if (ctx.hasSelection) {
    return [
      {
        label: '复制',
        disabled: false,
        onclick: () => void copySelection(ctx.selection),
      },
      {
        label: '全选',
        onclick: () => selectAll(ctx),
      },
    ];
  }
  return [];
}
