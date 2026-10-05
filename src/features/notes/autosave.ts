/**
 * 笔记自动保存的调度决策（纯函数，docs/18 B3 + S8）。
 *
 * 背景：S8 修的丢编辑缺陷发生在「防抖定时器到期时正好有一次保存还在飞」
 * 这条路径上——旧实现直接 `if (dirty && !saving) void save()`，保存中就
 * 空转且**无人重调度**，这一轮编辑永远不落盘且无任何提示。
 *
 * 把这个决策抽成纯函数，才能真正测到「保存中必须重排」这条不变量；
 * 此前笔记前端 0 用例（已逐一核对），丢编辑只能靠人工发现。
 */

/** 自动保存的外部状态快照。 */
export interface AutosaveState {
  /** 有未落盘的编辑。 */
  dirty: boolean;
  /** 有一次保存正在飞行。 */
  saving: boolean;
  /** 当前有选中的笔记（无选中时保存无意义）。 */
  hasActive: boolean;
}

/** 本次 tick 的决策。 */
export type AutosaveDecision =
  /** 触发一次保存。 */
  | { kind: 'save' }
  /**
   * 稍后重试：保存正在飞行，但仍有未落盘的编辑。
   * 不重排就会静默丢掉这一轮编辑——这是 S8 的核心修复。
   */
  | { kind: 'retry' }
  /** 无事可做。 */
  | { kind: 'idle'; reason: 'clean' | 'no-active' };

export const AUTOSAVE_DEBOUNCE_MS = 1500;

/**
 * 防抖定时器到期时的决策（唯一事实源）。
 *
 * 优先级：无选中 > 无改动 > 保存中（有改动则重排）> 保存。
 */
export function autosaveDecision(state: AutosaveState): AutosaveDecision {
  if (!state.hasActive) return { kind: 'idle', reason: 'no-active' };
  if (!state.dirty) return { kind: 'idle', reason: 'clean' };
  if (state.saving) return { kind: 'retry' };
  return { kind: 'save' };
}

/**
 * 卸载/关窗前是否需要提示用户（beforeunload 用）。
 * 仅「有未落盘编辑且没有保存在飞」时提示——保存已在飞则数据会落盘。
 */
export function needsUnloadPrompt(state: AutosaveState): boolean {
  return state.hasActive && state.dirty && !state.saving;
}
