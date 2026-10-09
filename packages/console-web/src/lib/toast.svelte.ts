/** 全站 toast（docs/20 §4.10）：右下角固定、5 秒自动收起、成功可撤销、失败带原因。
 *
 * 写在 .svelte.ts 里而不是 store 库——状态就是一个数组，runes 直接够用，
 * 不引第三方状态库是这个包唯一的依赖纪律（零运行时依赖）。
 */

export interface Toast {
  id: number;
  text: string;
  kind: 'ok' | 'fail';
  /** 撤销动作；单设备吊销有，批量吊销没有（凭据不可逆，见 docs/20 §4.4）。 */
  undo?: () => void;
}

const UNDO_WINDOW_MS = 5000;
const MAX_VISIBLE = 3;

export const toasts = $state<{ items: Toast[] }>({ items: [] });

let nextId = 1;
// 计时器簿记：只在模块内读写，从不进模板，所以要普通 Map 而不是 SvelteMap。
// eslint-disable-next-line svelte/prefer-svelte-reactivity
const timers = new Map<number, ReturnType<typeof setTimeout>>();

export function toast(text: string, kind: 'ok' | 'fail' = 'ok', undo?: () => void): number {
  const id = nextId++;
  toasts.items = [...toasts.items, { id, text, kind, undo }].slice(-MAX_VISIBLE);
  timers.set(
    id,
    setTimeout(() => dismiss(id), kind === 'fail' ? UNDO_WINDOW_MS * 2 : UNDO_WINDOW_MS),
  );
  return id;
}

/** 撤销后不弹第二次提示：toast 本身就是这次操作的反馈。 */
export function dismiss(id: number): void {
  const timer = timers.get(id);
  if (timer) {
    clearTimeout(timer);
    timers.delete(id);
  }
  toasts.items = toasts.items.filter((item) => item.id !== id);
}
