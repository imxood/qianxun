<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { call } from '../../lib/ipc';
  import { harness } from '../../stores/harness.svelte';
  import {
    formatHarnessStatus,
    type EnvProcLogLine,
    type LayaSettings,
    type LayaStatus,
    type MoliStatus,
    type QwenSettings,
    type QwenStatus,
    type Settings,
  } from '../../lib/ipc/contract';
  import { pinnedDshVersion } from '../../lib/utils/dsh-version';
  import { nav } from '../../stores/nav.svelte';

  // ---- DSH harness（沿用既有域逻辑） ----
  onMount(() => {
    void harness.refreshEnvironment();
    void harness.backfillLogs();
    void harness.refreshRecovery();
    void refreshServices();
    // Laya/QWen 侧车进程输出实时进右侧日志（Rust 侧 tools::relay_child_output 推送）。
    void listen<EnvProcLogLine>('env://log', (event) => {
      const { tool, stream, line } = event.payload;
      procLogs.push(`[${tool}.${stream}] ${line}`);
      if (procLogs.length > PROC_LOG_LIMIT) {
        procLogs.splice(0, procLogs.length - PROC_LOG_LIMIT);
      }
    }).then((unlisten) => (unlistenProcLog = unlisten));
  });

  let unlistenProcLog: UnlistenFn | null = null;
  onDestroy(() => unlistenProcLog?.());

  let actionError = $state('');
  let nodeError = $state('');
  let dshError = $state('');

  function installNode(): void {
    nodeError = '';
    void harness.installNode().catch((error: unknown) => {
      nodeError = error instanceof Error ? error.message : String(error);
    });
  }

  function installDsh(): void {
    dshError = '';
    void harness.install().catch((error: unknown) => {
      dshError = error instanceof Error ? error.message : String(error);
    });
  }

  async function start(): Promise<void> {
    actionError = '';
    try {
      await harness.start();
    } catch (error) {
      actionError = error instanceof Error ? error.message : String(error);
    }
  }

  async function stop(): Promise<void> {
    actionError = '';
    try {
      await harness.stop();
    } catch (error) {
      actionError = error instanceof Error ? error.message : String(error);
    }
  }

  async function restart(): Promise<void> {
    actionError = '';
    try {
      await harness.restart();
    } catch (error) {
      actionError = error instanceof Error ? error.message : String(error);
    }
  }

  const statusTone: Record<string, string> = {
    stopped: 'text-muted',
    starting: 'text-accent',
    ready: 'text-ok',
    restarting: 'text-accent',
    failed: 'text-danger',
  };

  const running = $derived(
    harness.status.phase === 'starting' ||
      harness.status.phase === 'ready' ||
      harness.status.phase === 'restarting',
  );

  // 外部 DSH 模式：千寻不内置、不托管 DSH，直接用用户全局安装的 dsh。
  // 这种模式下本机有没有 Node 与能不能启动无关（dsh 自带运行时）。
  const dshExternal = $derived(harness.environment?.dshSource === 'external');

  /** DSH 来源中文映射（bundled = 千寻托管；external = 用户全局安装）。 */
  const DSH_SOURCE_LABEL: Record<string, string> = {
    bundled: '内置托管',
    external: '全局安装',
  };

  /** 当前 DSH 来源文案（环境未探到时按内置托管展示）。 */
  const dshSourceLabel = $derived(
    DSH_SOURCE_LABEL[harness.environment?.dshSource ?? 'bundled'] ?? '内置托管',
  );

  /** 启动前提：DSH 就位且版本匹配；内置模式还要求本机有合格的 Node。 */
  const canStartDsh = $derived(
    Boolean(
      harness.environment &&
      harness.environment.dshInstalled &&
      harness.environment.dshVersionMatches &&
      (dshExternal || harness.environment.node),
    ),
  );

  /** failed 变体的收窄（union 上直接取 reason 过不了类型检查）。 */
  const failReason = $derived(harness.status.phase === 'failed' ? harness.status.reason : '');

  /**
   * 共享实例：千寻复用了本机已经在跑的 DSH（外部模式 + 已有 dsh web）。
   * 它不是千寻的进程——千寻不接管它的启停。
   */
  const sharedInstance = $derived(
    harness.status.phase === 'ready' && harness.status.shared ? harness.status : null,
  );

  // ---- Laya / QWen / Moli 环境状态 ----
  let layaStatus = $state<LayaStatus | null>(null);
  let qwenStatus = $state<QwenStatus | null>(null);
  let moliStatus = $state<MoliStatus | null>(null);

  let layaForm = $state<LayaSettings>({ serverExe: '', modelDir: '', port: 10230, threads: 8 });
  let qwenForm = $state<QwenSettings>({ strataHome: '', config: '', port: 23661 });
  let moliBinaryPath = $state('');

  let layaBusy = $state<'' | 'starting' | 'stopping' | 'saving'>('');
  let qwenBusy = $state<'' | 'starting' | 'stopping' | 'saving'>('');
  let moliBusy = $state<'' | 'saving' | 'probing'>('');
  let layaError = $state('');
  let qwenError = $state('');
  let moliError = $state('');
  let layaConfigOpen = $state(false);
  let qwenConfigOpen = $state(false);
  let moliConfigOpen = $state(false);

  const MOLI_SOURCE_LABEL: Record<string, string> = {
    custom: '自备',
    managed: '受管',
    path: 'PATH',
    none: '未安装',
  };

  async function refreshSettingsForms(): Promise<void> {
    try {
      const settings = await call<Settings>('settings_get');
      layaForm = { ...settings.tools.laya };
      qwenForm = { ...settings.tools.qwen };
      moliBinaryPath = settings.tools.moli.binaryPath;
    } catch (error) {
      console.error('读取设置失败', error);
    }
  }

  async function refreshServices(): Promise<void> {
    await refreshSettingsForms();
    const results = await Promise.allSettled([
      call<LayaStatus>('laya_status'),
      call<QwenStatus>('qwen_status'),
      call<MoliStatus>('moli_status'),
    ]);
    if (results[0]?.status === 'fulfilled') layaStatus = results[0].value;
    if (results[1]?.status === 'fulfilled') qwenStatus = results[1].value;
    if (results[2]?.status === 'fulfilled') moliStatus = results[2].value;
  }

  async function layaAction(action: 'laya_start' | 'laya_stop'): Promise<void> {
    layaError = '';
    layaBusy = action === 'laya_start' ? 'starting' : 'stopping';
    try {
      layaStatus = await call<LayaStatus>(action);
    } catch (error) {
      layaError = error instanceof Error ? error.message : String(error);
    } finally {
      layaBusy = '';
    }
  }

  async function qwenAction(action: 'qwen_start' | 'qwen_stop'): Promise<void> {
    qwenError = '';
    qwenBusy = action === 'qwen_start' ? 'starting' : 'stopping';
    try {
      qwenStatus = await call<QwenStatus>(action);
    } catch (error) {
      qwenError = error instanceof Error ? error.message : String(error);
    } finally {
      qwenBusy = '';
    }
  }

  async function saveLaya(): Promise<void> {
    layaError = '';
    layaBusy = 'saving';
    try {
      await call<Settings>('settings_update', {
        patch: { tools: { laya: { ...layaForm } } },
      });
      layaStatus = await call<LayaStatus>('laya_status');
    } catch (error) {
      layaError = error instanceof Error ? error.message : String(error);
    } finally {
      layaBusy = '';
    }
  }

  async function saveQwen(): Promise<void> {
    qwenError = '';
    qwenBusy = 'saving';
    try {
      await call<Settings>('settings_update', {
        patch: { tools: { qwen: { ...qwenForm } } },
      });
      qwenStatus = await call<QwenStatus>('qwen_status');
    } catch (error) {
      qwenError = error instanceof Error ? error.message : String(error);
    } finally {
      qwenBusy = '';
    }
  }

  async function saveMoli(): Promise<void> {
    moliError = '';
    moliBusy = 'saving';
    try {
      await call<Settings>('settings_update', {
        patch: { tools: { moli: { binaryPath: moliBinaryPath } } },
      });
      moliBusy = 'probing';
      moliStatus = await call<MoliStatus>('moli_status');
    } catch (error) {
      moliError = error instanceof Error ? error.message : String(error);
    } finally {
      moliBusy = '';
    }
  }

  // 轻量轮询：服务在千寻之外被启停也能反映出来。仅环境页可见时轮询
  // （keep-alive 常挂载，切走即停），且命令已 async 化，绝不阻塞主线程。
  $effect(() => {
    if (nav.page !== 'env') return;
    const timer = setInterval(() => {
      void call<LayaStatus>('laya_status')
        .then((status) => (layaStatus = status))
        .catch(() => {});
      void call<QwenStatus>('qwen_status')
        .then((status) => (qwenStatus = status))
        .catch(() => {});
    }, 10_000);
    return () => clearInterval(timer);
  });

  // ---- 日志 ----
  /** 侧车进程输出上限：超过丢弃最旧的（与 harness 的 LOG_LIMIT 同量级）。 */
  const PROC_LOG_LIMIT = 2000;
  let procLogs = $state<string[]>([]);
  /** DSH 托管日志 + 侧车进程输出合并展示（侧车行自带 [tool.stream] 前缀）。 */
  const displayLogs = $derived([...harness.logs, ...procLogs]);
  let logBox = $state<HTMLDivElement | null>(null);
  let pinnedToBottom = true;

  $effect(() => {
    void displayLogs.length;
    if (logBox && pinnedToBottom) logBox.scrollTop = logBox.scrollHeight;
  });

  // ---- 状态点着色 ----
  const dot = (tone: 'ok' | 'warn' | 'off' | 'busy'): string =>
    ({
      ok: 'bg-emerald-500',
      warn: 'bg-amber-500',
      off: 'bg-muted/40',
      busy: 'bg-accent animate-pulse',
    })[tone];

  // Laya 卡状态推导：running→ok；未配置→warn；否则 off
  const layaTone = $derived<'ok' | 'warn' | 'off' | 'busy'>(
    layaBusy === 'starting'
      ? 'busy'
      : layaStatus?.running
        ? 'ok'
        : layaStatus === null
          ? 'off'
          : 'warn',
  );
  const layaLine = $derived(
    layaBusy === 'starting'
      ? '启动中…（首次加载模型约 10s）'
      : layaStatus?.running
        ? `运行中 · :${layaStatus.port} · ${layaStatus.threads} 线程${layaStatus.pid ? '' : ' · 外部实例'}`
        : '已停止',
  );
  const qwenTone = $derived<'ok' | 'warn' | 'off' | 'busy'>(
    qwenBusy === 'starting'
      ? 'busy'
      : qwenStatus?.ready
        ? 'ok'
        : qwenStatus?.running
          ? 'busy'
          : 'warn',
  );
  const qwenLine = $derived(
    qwenBusy === 'starting'
      ? '启动中…（内存载入 1-3 分钟，期间卡顿属正常）'
      : qwenStatus?.ready
        ? `就绪 · :${qwenStatus.port}`
        : qwenStatus?.running
          ? '端口已监听 · 模型加载中'
          : '已停止',
  );
</script>

<!-- 左列四张环境卡（纵向滚动），右侧日志铺满全高。 -->
<section class="flex h-full flex-col gap-3 p-4">
  <header class="shrink-0">
    <h1 class="qx-page-title">环境</h1>
  </header>

  <div class="flex min-h-0 flex-1 gap-3">
    <aside class="flex w-[24rem] shrink-0 flex-col gap-2.5 overflow-y-auto pr-1">
      <!-- DSH -->
      <div class="qx-card p-3.5">
        <div class="flex h-7 items-center justify-between">
          <span class="flex items-center gap-2 text-sm font-medium">
            <span
              class="size-1.5 rounded-full {statusTone[harness.status.phase] ??
                'text-muted'} bg-current"
              aria-hidden="true"
            ></span>
            DSH
          </span>
          <span class="flex items-center gap-1.5">
            {#if running}
              <button
                class="qx-btn qx-btn-sm qx-btn-primary"
                disabled={harness.restarting}
                data-testid="harness-restart"
                onclick={() => void restart()}
              >
                {harness.restarting ? '重启中…' : '重启'}
              </button>
              <button
                class="qx-btn qx-btn-sm qx-btn-outline"
                disabled={harness.restarting || harness.status.phase === 'starting'}
                onclick={() => void stop()}
              >
                停止
              </button>
            {:else}
              <button
                class="qx-btn qx-btn-sm qx-btn-primary"
                disabled={harness.starting || !canStartDsh}
                data-testid="harness-start"
                onclick={() => void start()}
              >
                {harness.starting ? '启动中…' : '启动'}
              </button>
            {/if}
          </span>
        </div>
        <p class="truncate text-xs text-muted" title={failReason}>
          {formatHarnessStatus(harness.status)}
          {#if harness.environment?.dshVersion}
            · v{harness.environment.dshVersion}
          {/if}
        </p>
        <p class="mt-1 text-xs text-muted" data-testid="dsh-source-line">
          来源：{dshSourceLabel}
        </p>
        {#if sharedInstance}
          <p class="mt-1 text-xs text-muted" data-testid="dsh-shared-line">
            已复用本机正在运行的 DSH（{sharedInstance.origin}
            {sharedInstance.pid
              ? `· pid ${sharedInstance.pid}`
              : ''}）：千寻不接管它的启停——「停止」只是断开连接，不会结束那个进程。
          </p>
        {/if}
        {#if harness.status.phase === 'failed'}
          <p class="mt-1.5 text-xs text-danger">{failReason}</p>
          <div class="mt-2 flex flex-wrap gap-1.5">
            {#if harness.recovery.snapshotAvailable}
              <button
                class="qx-btn qx-btn-sm qx-btn-outline"
                disabled={harness.recovering}
                data-testid="harness-recover"
                onclick={() =>
                  harness
                    .recoverKnownGood()
                    .catch(
                      (error: unknown) =>
                        (actionError = error instanceof Error ? error.message : String(error)),
                    )}
              >
                {harness.recovering ? '恢复中…' : '恢复上次可用配置'}
              </button>
            {/if}
            <button
              class="qx-btn qx-btn-sm qx-btn-outline"
              disabled={harness.recovering}
              data-testid="harness-safe-mode"
              onclick={() =>
                harness
                  .safeModeStart()
                  .catch(
                    (error: unknown) =>
                      (actionError = error instanceof Error ? error.message : String(error)),
                  )}
            >
              安全模式
            </button>
          </div>
        {/if}
        {#if harness.recovery.safeMode && harness.status.phase === 'ready'}
          <p
            class="mt-1.5 rounded-lg border border-warning/40 bg-warning/10 px-2 py-1 text-xs text-warning"
            data-testid="safe-mode-badge"
          >
            安全模式中：插件变更冻结，重启 DSH 退出。
          </p>
        {/if}
        {#if harness.environment && !harness.environment.node && !dshExternal}
          <div class="mt-2 flex items-center justify-between gap-2">
            <p class="text-xs text-muted">未检测到 Node</p>
            <button
              class="qx-btn qx-btn-sm qx-btn-primary"
              disabled={harness.installing}
              onclick={installNode}
            >
              {harness.installing ? '安装中…' : `安装 v${harness.environment.bundledNodeVersion}`}
            </button>
          </div>
          {#if nodeError}
            <p class="mt-1 text-xs text-danger">{nodeError}</p>
          {/if}
        {/if}
        {#if harness.environment && dshExternal}
          <p class="mt-2 text-xs text-muted">当前使用全局 DSH（千寻不托管安装与升级）</p>
          {#if harness.environment.dshExternalPath}
            <p
              class="truncate font-mono text-xs text-muted"
              title={harness.environment.dshExternalPath}
            >
              {harness.environment.dshExternalPath}
              {#if harness.environment.dshVersion}
                · v{harness.environment.dshVersion}
              {/if}
            </p>
          {:else}
            <p class="mt-1 text-xs text-warning">
              未找到全局安装的 DSH：请确认 dsh 已在 PATH 中，或用 QIANXUN_DSH_BIN 指向它。
            </p>
          {/if}
          {#if harness.environment.dshExternalPath && !harness.environment.dshVersionMatches}
            <p class="mt-1 text-xs text-warning">
              版本不兼容：要求 {pinnedDshVersion(harness.environment.installSpec)}，当前 {harness
                .environment.dshVersion ?? '未知'}
            </p>
          {/if}
        {:else if harness.environment && !harness.environment.dshInstalled}
          <div class="mt-2 flex items-center justify-between gap-2">
            <p class="text-xs text-muted">DSH 未安装</p>
            <button
              class="qx-btn qx-btn-sm qx-btn-primary"
              disabled={harness.installing || !harness.environment.node}
              onclick={installDsh}
            >
              {harness.installing ? '安装中…' : '安装'}
            </button>
          </div>
          {#if dshError}
            <p class="mt-1 text-xs text-danger">{dshError}</p>
          {/if}
        {:else if harness.environment && !harness.environment.dshVersionMatches}
          <div class="mt-2 flex items-center justify-between gap-2">
            <p class="text-xs text-warning">
              版本不匹配：要求 {pinnedDshVersion(harness.environment.installSpec)}，当前 {harness
                .environment.dshVersion ?? '?'}
            </p>
            <button
              class="qx-btn qx-btn-sm qx-btn-primary"
              disabled={harness.installing || !harness.environment.node}
              onclick={installDsh}
            >
              {harness.installing ? '升级中…' : '升级'}
            </button>
          </div>
          {#if dshError}
            <p class="mt-1 text-xs text-danger">{dshError}</p>
          {/if}
        {/if}
        {#if actionError}
          <p class="mt-1.5 text-xs text-danger">{actionError}</p>
        {/if}
      </div>

      <!-- Laya -->
      <div class="qx-card p-3.5">
        <div class="flex h-7 items-center justify-between">
          <span class="flex items-center gap-2 text-sm font-medium">
            <span class="size-1.5 rounded-full {dot(layaTone)}" aria-hidden="true"></span>
            Laya
          </span>
          <span class="flex items-center gap-1.5">
            {#if layaStatus?.running}
              <button
                class="qx-btn qx-btn-sm qx-btn-outline"
                disabled={layaBusy !== ''}
                onclick={() => void layaAction('laya_stop')}
              >
                {layaBusy === 'stopping' ? '停止中…' : '停止'}
              </button>
            {:else}
              <button
                class="qx-btn qx-btn-sm qx-btn-primary"
                disabled={layaBusy !== ''}
                data-testid="laya-start"
                onclick={() => void layaAction('laya_start')}
              >
                {layaBusy === 'starting' ? '启动中…' : '启动'}
              </button>
            {/if}
            <button
              class="qx-btn qx-btn-sm qx-btn-outline"
              onclick={() => (layaConfigOpen = !layaConfigOpen)}
              aria-expanded={layaConfigOpen}
            >
              {layaConfigOpen ? '收起' : '配置'}
            </button>
          </span>
        </div>
        <p class="truncate text-xs text-muted" data-testid="laya-line">{layaLine}</p>
        {#if layaStatus && !layaStatus.running}
          <p class="text-xs text-muted">
            {#if !layaStatus.serverExe && !layaForm.serverExe}
              可执行文件未找到 — 在配置中指定
            {:else if !layaStatus.modelDir && !layaForm.modelDir}
              模型目录未找到 — 在配置中指定
            {/if}
          </p>
        {/if}
        {#if layaStatus?.healthCheckpoint}
          <p class="truncate text-xs text-muted" title={layaStatus.healthCheckpoint}>
            模型：{layaStatus.healthCheckpoint}
          </p>
        {/if}
        {#if layaConfigOpen}
          <div class="mt-2 space-y-1.5 border-t border-line/60 pt-2.5">
            <label class="block">
              <span class="text-xs text-muted">模型目录</span>
              <input
                class="qx-input mt-0.5 w-full font-mono text-xs"
                bind:value={layaForm.modelDir}
                placeholder="含 laya.onnx 的目录；空 = 自动探测"
              />
            </label>
            <label class="block">
              <span class="text-xs text-muted">可执行文件</span>
              <input
                class="qx-input mt-0.5 w-full font-mono text-xs"
                bind:value={layaForm.serverExe}
                placeholder="laya-server.exe；空 = 自动探测"
              />
            </label>
            <div class="flex gap-1.5">
              <label class="block w-28">
                <span class="text-xs text-muted">端口</span>
                <input
                  class="qx-input mt-0.5 w-full font-mono text-xs"
                  type="number"
                  bind:value={layaForm.port}
                  min="1024"
                  max="65535"
                />
              </label>
              <label class="block w-24">
                <span class="text-xs text-muted">线程</span>
                <input
                  class="qx-input mt-0.5 w-full font-mono text-xs"
                  type="number"
                  bind:value={layaForm.threads}
                  min="1"
                  max="32"
                />
              </label>
              <button
                class="qx-btn qx-btn-sm qx-btn-primary mt-[18px]"
                disabled={layaBusy === 'saving'}
                onclick={() => void saveLaya()}
              >
                {layaBusy === 'saving' ? '保存中…' : '保存'}
              </button>
            </div>
          </div>
        {/if}
        {#if layaStatus?.detail}
          <p class="text-xs text-muted">{layaStatus.detail}</p>
        {/if}
        {#if layaError}
          <p class="text-xs text-danger" role="alert">{layaError}</p>
        {/if}
      </div>

      <!-- Qwen3.8-Flash-Next（Strata 本地引擎） -->
      <div class="qx-card p-3.5">
        <div class="flex h-7 items-center justify-between">
          <span class="flex items-center gap-2 text-sm font-medium">
            <span class="size-1.5 rounded-full {dot(qwenTone)}" aria-hidden="true"></span>
            Qwen3.8-Flash-Next
          </span>
          <span class="flex items-center gap-1.5">
            {#if qwenStatus?.running}
              <button
                class="qx-btn qx-btn-sm qx-btn-outline"
                disabled={qwenBusy !== ''}
                onclick={() => void qwenAction('qwen_stop')}
              >
                {qwenBusy === 'stopping' ? '停止中…' : '停止'}
              </button>
            {:else}
              <button
                class="qx-btn qx-btn-sm qx-btn-primary"
                disabled={qwenBusy !== '' || !qwenForm.strataHome}
                data-testid="qwen-start"
                onclick={() => void qwenAction('qwen_start')}
              >
                {qwenBusy === 'starting' ? '启动中…' : '启动'}
              </button>
            {/if}
            <button
              class="qx-btn qx-btn-sm qx-btn-outline"
              onclick={() => (qwenConfigOpen = !qwenConfigOpen)}
              aria-expanded={qwenConfigOpen}
            >
              {qwenConfigOpen ? '收起' : '配置'}
            </button>
          </span>
        </div>
        <p class="truncate text-xs text-muted" data-testid="qwen-line">
          {qwenLine}
          {#if qwenStatus?.running}
            <span class="text-muted">· {qwenStatus.ready ? '' : '外部'}</span>
          {/if}
        </p>
        {#if qwenConfigOpen}
          <div class="mt-2 space-y-1.5 border-t border-line/60 pt-2.5">
            <label class="block">
              <span class="text-xs text-muted">Strata 程序目录（含 .venv 与 serve）</span>
              <input
                class="qx-input mt-0.5 w-full font-mono text-xs"
                bind:value={qwenForm.strataHome}
                placeholder="如 E:\git\ai\Strata"
              />
            </label>
            <label class="block">
              <span class="text-xs text-muted">引擎配置（相对程序目录或绝对路径）</span>
              <input
                class="qx-input mt-0.5 w-full font-mono text-xs"
                bind:value={qwenForm.config}
                placeholder="D:\AI\Strata\strata-iq2_xs.json"
              />
            </label>
            <div class="flex gap-1.5">
              <label class="block w-28">
                <span class="text-xs text-muted">端口</span>
                <input
                  class="qx-input mt-0.5 w-full font-mono text-xs"
                  type="number"
                  bind:value={qwenForm.port}
                  min="1024"
                  max="65535"
                />
              </label>
              <button
                class="qx-btn qx-btn-sm qx-btn-primary mt-[18px]"
                disabled={qwenBusy === 'saving'}
                onclick={() => void saveQwen()}
              >
                {qwenBusy === 'saving' ? '保存中…' : '保存'}
              </button>
            </div>
          </div>
        {/if}
        {#if qwenStatus?.detail}
          <p class="text-xs text-muted">{qwenStatus.detail}</p>
        {/if}
        {#if qwenError}
          <p class="text-xs text-danger" role="alert">{qwenError}</p>
        {/if}
      </div>

      <!-- Moli -->
      <div class="qx-card p-3.5">
        <div class="flex h-7 items-center justify-between">
          <span class="flex items-center gap-2 text-sm font-medium">
            <span
              class="size-1.5 rounded-full {moliStatus?.installed
                ? 'bg-emerald-500'
                : moliStatus === null
                  ? 'bg-muted/40'
                  : 'bg-amber-500'}"
              aria-hidden="true"
            ></span>
            Moli
          </span>
          <span class="flex items-center gap-1.5">
            <button
              class="qx-btn qx-btn-sm qx-btn-outline"
              disabled={moliBusy !== ''}
              onclick={() => {
                moliBusy = 'probing';
                void call<MoliStatus>('moli_status')
                  .then((status) => (moliStatus = status))
                  .catch(() => {})
                  .finally(() => (moliBusy = ''));
              }}
            >
              {moliBusy === 'probing' ? '检测中…' : '重新检测'}
            </button>
            <button
              class="qx-btn qx-btn-sm qx-btn-outline"
              onclick={() => (moliConfigOpen = !moliConfigOpen)}
              aria-expanded={moliConfigOpen}
            >
              {moliConfigOpen ? '收起' : '配置'}
            </button>
          </span>
        </div>
        <p class="truncate text-xs text-muted" data-testid="moli-line">
          {#if moliStatus?.installed}
            可用 · v{moliStatus.version ?? '?'} · {MOLI_SOURCE_LABEL[moliStatus.source] ??
              moliStatus.source}
          {:else if moliStatus}
            未安装 — 配置自备路径或安装到 PATH
          {:else}
            检测中…
          {/if}
        </p>
        {#if moliStatus?.path}
          <p class="truncate font-mono text-xs text-muted" title={moliStatus.path}>
            {moliStatus.path}
          </p>
        {/if}
        {#if moliConfigOpen}
          <div class="mt-2 space-y-1.5 border-t border-line/60 pt-2.5">
            <label class="block">
              <span class="text-xs text-muted">自备路径（空 = 受管安装 / PATH）</span>
              <input
                class="qx-input mt-0.5 w-full font-mono text-xs"
                bind:value={moliBinaryPath}
                placeholder="moli.exe 绝对路径"
              />
            </label>
            <button
              class="qx-btn qx-btn-sm qx-btn-primary"
              disabled={moliBusy !== ''}
              onclick={() => void saveMoli()}
            >
              {moliBusy === 'saving' ? '保存中…' : '保存并检测'}
            </button>
          </div>
        {/if}
        {#if moliError}
          <p class="text-xs text-danger" role="alert">{moliError}</p>
        {/if}
      </div>
    </aside>

    <!-- 日志 -->
    <div class="qx-card flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div
        class="flex h-8 shrink-0 items-center justify-between border-b border-line/70 px-3 text-xs"
      >
        <span class="font-medium text-muted">日志</span>
        <span class="font-mono tabular-nums text-muted/70">{displayLogs.length}</span>
      </div>
      <div
        bind:this={logBox}
        class="min-h-0 flex-1 overflow-y-auto p-3 font-mono text-xs leading-5"
        onscroll={() => {
          if (!logBox) return;
          pinnedToBottom = logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 24;
        }}
      >
        {#if displayLogs.length === 0}
          <p class="text-muted/70">暂无日志</p>
        {:else}
          <!-- 只渲染尾部 150 行：store 保留 2000 行，DSH 启动刷屏时每条
               IPC 都触发 keep-alive 页的列表 diff，全量渲染是卡顿源之一。 -->
          {#each displayLogs.slice(-150) as line, index (index)}
            <div class="whitespace-pre-wrap break-all">{line}</div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
</section>
