/**
 * 托管域前端状态：supervisor 状态镜像 + 日志环形缓冲 + 动作。
 *
 * 事件订阅只建一次（单例 store），页面只读状态不碰事件。
 * 状态初始值向 Rust 要一次（harness_status），随后由事件驱动。
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { call } from '../lib/ipc';
import type { HarnessEnvironment, HarnessEvent, HarnessStatus } from '../lib/ipc/contract';

const LOG_LIMIT = 2000;

class HarnessStore {
  status: HarnessStatus = $state({ phase: 'stopped' });
  environment: HarnessEnvironment | null = $state(null);
  environmentLoading = $state(false);
  starting = $state(false);
  installing = $state(false);
  logs: string[] = $state([]);
  /** 进程仍在跑（starting/ready/restarting）时置 true，控制按钮可用性。 */
  busy = $state(false);
  private wired = false;
  private unlisten: UnlistenFn | null = null;

  async wire(): Promise<void> {
    if (this.wired) return;
    this.wired = true;
    this.unlisten = await listen<HarnessEvent>('harness://event', (event) => {
      this.ingest(event.payload);
    });
    // 已运行实例的当前状态（事件只覆盖未来的变化）。
    try {
      this.status = await call<HarnessStatus>('harness_status');
    } catch {
      // 状态拿不到不阻塞页面：显示默认的「未运行」。
    }
  }

  dispose(): void {
    this.unlisten?.();
    this.unlisten = null;
    this.wired = false;
  }

  private ingest(event: HarnessEvent): void {
    if (event.kind === 'status') {
      // kind 只是判别字段，剥掉后剩余部分即 HarnessStatus。
      const { kind: _kind, ...status } = event;
      this.status = status;
      this.busy =
        status.phase === 'starting' || status.phase === 'ready' || status.phase === 'restarting';
      if (status.phase === 'ready') this.starting = false;
      if (status.phase === 'failed' || status.phase === 'stopped') this.starting = false;
      return;
    }
    this.logs.push(`${event.stream === 'stderr' ? '⚠ ' : ''}${event.line}`);
    if (this.logs.length > LOG_LIMIT) this.logs.splice(0, this.logs.length - LOG_LIMIT);
  }

  async refreshEnvironment(): Promise<void> {
    this.environmentLoading = true;
    try {
      this.environment = await call<HarnessEnvironment>('harness_environment');
    } finally {
      this.environmentLoading = false;
    }
  }

  async start(): Promise<void> {
    this.starting = true;
    try {
      await call<string>('harness_start');
    } catch (error) {
      this.starting = false;
      throw error;
    }
  }

  async stop(): Promise<void> {
    await call('harness_stop');
  }

  async install(): Promise<void> {
    this.installing = true;
    try {
      await call('harness_install');
      await this.refreshEnvironment();
    } finally {
      this.installing = false;
    }
  }

  /** 一键安装千寻自带的 Node（下载 + SHA-256 校验 + 解压）。 */
  async installNode(): Promise<void> {
    this.installing = true;
    try {
      await call('harness_install_node');
      await this.refreshEnvironment();
    } finally {
      this.installing = false;
    }
  }

  /** 打开日志面板时回填历史（晚订阅不空白）。 */
  async backfillLogs(): Promise<void> {
    if (this.logs.length > 0) return;
    try {
      const history = await call<Array<{ stream: string; line: string }>>('harness_log');
      for (const { line } of history) this.logs.push(line);
    } catch {
      // 历史拿不到就用事件流从头开始。
    }
  }
}

export const harness = new HarnessStore();
