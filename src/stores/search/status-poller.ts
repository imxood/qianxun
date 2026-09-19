/**
 * 扫描状态轮询器（汇总 §3.2 / 04 §S1）：
 *
 * 搜索域前端用 `setInterval(300ms)` 轮询 `search_status` 直到 `scanning=false`
 * 再补一次终态就停表。原实现把这个 setInterval 直接放进 `SearchStore.ensurePolling`
 * 私有方法，单测必须 mock 整个 store 才能覆盖 "scanning=false 后立即停表"。
 *
 * 抽成独立小类后可单测：注入 `isActive()` getter + `tick()` 回调，启动 / 停止
 * 接口都明确。失败回调兜底不阻断 UI（汇总 §03 P13 周边）。
 */

export interface StatusPollerOptions {
  /** 每隔多少毫秒 tick 一次。 */
  intervalMs: number;
  /** 当前是否仍在扫描（false 时下一 tick 触发 `onStop` 后停表）。 */
  isActive: () => boolean;
  /** 每 tick 调一次（一般为 `search.refreshStatus()`）。 */
  onTick: () => Promise<unknown> | unknown;
  /** 轮询真正停表后调一次（用于补一次终态）。 */
  onStop?: () => void;
  /** 自检钩子：在测试里用 fake timers 时不直接依赖 Node setInterval。 */
  setIntervalFn?: (fn: () => void, ms: number) => ReturnType<typeof setInterval>;
  clearIntervalFn?: (handle: ReturnType<typeof setInterval>) => void;
}

export class StatusPoller {
  private handle: ReturnType<typeof setInterval> | null = null;
  private stopped = false;

  constructor(private readonly opts: StatusPollerOptions) {}

  /** 启动轮询；已启动则幂等。 */
  start(): void {
    if (this.handle !== null) return;
    this.stopped = false;
    const setI: (fn: () => void, ms: number) => ReturnType<typeof setInterval> =
      this.opts.setIntervalFn ?? ((fn, ms) => setInterval(fn, ms));
    this.handle = setI(() => this.tick(), this.opts.intervalMs);
  }

  /** 显式停表。 */
  stop(): void {
    this.stopped = true;
    if (this.handle === null) return;
    const clearI: (h: ReturnType<typeof setInterval>) => void =
      this.opts.clearIntervalFn ?? ((h) => clearInterval(h));
    clearI(this.handle);
    this.handle = null;
    this.opts.onStop?.();
  }

  /** 测试用：手动驱动一次循环体。 */
  async tick(): Promise<void> {
    if (this.stopped) return;
    try {
      await this.opts.onTick();
    } catch {
      // 轮询失败不打断 UI，下轮再试（汇总 §03 P13 语义）。
    }
    if (!this.opts.isActive()) {
      // 补一次终态（如果 onTick 已经是终态了也无害）然后停表。
      try {
        await this.opts.onTick();
      } catch {
        /* ignore */
      }
      this.stop();
    }
  }

  get running(): boolean {
    return this.handle !== null && !this.stopped;
  }
}
