/**
 * 跨页轻量 toast 队列（汇总 §3.16 locate.ts 静默失败修复）：
 *
 * FilesPage / GrepPage / RootBar 之前对 locate.ts 调用的失败全 `.catch(() => {})`，
 * 用户无反馈（违反 docs/03 §8）。DiskScan 自带局部 toast，但其它页缺位。
 *
 * 单例 toast 队列：后入覆盖（同一时刻只显示一条）；4 秒自动消失；可携带
 * 单按钮「行动」回调（例如"重试"）。
 */

export interface Toast {
  text: string;
  action?: { label: string; run: () => void };
}

class ToastStore {
  current: Toast | null = $state(null);
  private timer: ReturnType<typeof setTimeout> | null = null;

  show(text: string, action?: Toast['action'], durationMs = 4000): void {
    if (this.timer) clearTimeout(this.timer);
    this.current = { text, action };
    this.timer = setTimeout(() => this.dismiss(), durationMs);
  }

  dismiss(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.current = null;
  }

  dispose(): void {
    this.dismiss();
  }
}

export const toast = new ToastStore();
