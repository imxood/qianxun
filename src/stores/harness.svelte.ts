/**
 * 托管域前端状态：supervisor 状态镜像 + 日志环形缓冲 + 动作。
 *
 * 事件订阅只建一次（单例 store），页面只读状态不碰事件。
 * 状态初始值向 Rust 要一次（harness_status），随后由事件驱动。
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { call } from '../lib/ipc';
import type {
  HarnessEnvironment,
  HarnessEvent,
  HarnessStatus,
  InstallProgress,
  RecoveryStatus,
} from '../lib/ipc/contract';

const LOG_LIMIT = 2000;

class HarnessStore {
  status: HarnessStatus = $state({ phase: 'stopped' });
  /**
   * DSH 回环代理地址（iframe 同站承载：Strict cookie 跨站不可携带，
   * cookie 留服务端）。进程内绑定一次终身不变；null = 代理未监听成功。
   */
  proxyUrl: string | null = $state(null);
  environment: HarnessEnvironment | null = $state(null);
  environmentLoading = $state(false);
  starting = $state(false);
  /** 手动重启动作进行中（按钮禁用与文案用；进程相位仍由事件驱动）。 */
  restarting = $state(false);
  installing = $state(false);
  /** 最近一次安装进度事件（UI 目前只展示日志；安装结束即清空）。 */
  installProgress: InstallProgress | null = $state(null);
  logs: string[] = $state([]);
  /** 进程仍在跑（starting/ready/restarting）时置 true，控制按钮可用性。 */
  busy = $state(false);
  /**
   * 启动失败自救状态（08 设计 §11.5）：快照是否存在（失败态「恢复」按钮）、
   * 安全 profile 是否就绪、当前是否跑在安全模式。失败/成功相位变化后刷新。
   */
  recovery: RecoveryStatus = $state({
    snapshotAvailable: false,
    safeProfileReady: false,
    safeMode: false,
  });
  /** 手动恢复/安全模式动作进行中（按钮禁用用）。 */
  recovering = $state(false);
  private wired = false;
  private unlisten: UnlistenFn | null = null;
  private unlistenProgress: UnlistenFn | null = null;

  async wire(): Promise<void> {
    if (this.wired) return;
    this.wired = true;
    this.unlisten = await listen<HarnessEvent>('harness://event', (event) => {
      this.ingest(event.payload);
    });
    this.unlistenProgress = await listen<InstallProgress>('harness://install-progress', (event) => {
      this.ingestProgress(event.payload);
    });
    // 已运行实例的当前状态（事件只覆盖未来的变化）。
    try {
      this.status = await call<HarnessStatus>('harness_status');
    } catch {
      // 状态拿不到不阻塞页面：显示默认的「未运行」。
    }
    await this.refreshProxyUrl();
    await this.refreshRecovery();
  }

  dispose(): void {
    this.unlisten?.();
    this.unlistenProgress?.();
    this.unlisten = null;
    this.unlistenProgress = null;
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
      // 快照/安全模式是文件与启动路径的事实：相位变化后刷新一次
      // （失败态按钮与安全模式徽标的数据源；fire-and-forget）。
      void this.refreshRecovery();
      return;
    }
    this.logs.push(`${event.stream === 'stderr' ? '⚠ ' : ''}${event.line}`);
    if (this.logs.length > LOG_LIMIT) this.logs.splice(0, this.logs.length - LOG_LIMIT);
  }

  private ingestProgress(progress: InstallProgress): void {
    if (progress.stage === 'done') {
      this.installProgress = null;
      return;
    }
    if (progress.stage === 'dsh-packages') {
      // pnpm 每行只带部分维度：缺失的维度沿用上一个事件，展示层免判空。
      const previous = this.installProgress;
      if (previous?.stage === 'dsh-packages') {
        this.installProgress = {
          ...progress,
          resolved: progress.resolved ?? previous.resolved,
          totalHint: progress.totalHint ?? previous.totalHint,
        };
        return;
      }
    }
    this.installProgress = progress;
  }

  async refreshEnvironment(): Promise<void> {
    this.environmentLoading = true;
    try {
      this.environment = await call<HarnessEnvironment>('harness_environment');
    } finally {
      this.environmentLoading = false;
    }
  }

  /** 补拉代理地址（wire 与 setup 的监听竞态兜底；幂等）。 */
  async refreshProxyUrl(): Promise<void> {
    try {
      this.proxyUrl = await call<string | null>('harness_proxy_url');
    } catch {
      // 拿不到保持 null：DSH 页会给出明确提示，不静默回退直连。
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

  /**
   * 重启 DSH：运行中先停后启，未运行等价启动。动作期间 restarting
   * 置 true，按钮据它禁用（进程相位由状态事件另行驱动）。
   */
  async restart(): Promise<void> {
    this.restarting = true;
    try {
      await call<string>('harness_restart');
    } finally {
      this.restarting = false;
    }
  }

  async install(): Promise<void> {
    this.installing = true;
    try {
      await call('harness_install');
      await this.refreshEnvironment();
    } finally {
      this.installing = false;
      this.installProgress = null;
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
      this.installProgress = null;
    }
  }

  /** 拉取启动失败自救状态（快照/安全 profile/是否安全模式）。 */
  async refreshRecovery(): Promise<void> {
    try {
      this.recovery = await call<RecoveryStatus>('harness_recovery_status');
    } catch {
      // 拿不到保持上次值：按钮显示条件宁可保守（不显示）。
    }
  }

  /**
   * 手动恢复到上次能启动的配置（08 设计 §11.5）：停机 → 恢复快照 →
   * 重启默认 profile。返回被卸下的插件名（前端展示恢复摘要）。
   */
  async recoverKnownGood(): Promise<string[]> {
    this.recovering = true;
    try {
      const removed = await call<string[]>('harness_recover_known_good');
      await this.refreshRecovery();
      return removed;
    } finally {
      this.recovering = false;
    }
  }

  /** 以最小安全 profile 启动（逃生舱；默认 profile 不受影响）。 */
  async safeModeStart(): Promise<void> {
    this.recovering = true;
    try {
      await call<string>('harness_safe_mode_start');
      await this.refreshRecovery();
    } finally {
      this.recovering = false;
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
