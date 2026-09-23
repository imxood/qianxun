<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from '../../lib/ipc';
  import { harness } from '../../stores/harness.svelte';
  import {
    formatHarnessStatus,
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
  });

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

  /** failed 变体的收窄（union 上直接取 reason 过不了类型检查）。 */
  const failReason = $derived(harness.status.phase === 'failed' ? harness.status.reason : '');

  // ---- Laya / QWen / Moli 环境状态 ----
  let layaStatus = $state<LayaStatus | null>(null);
  let qwenStatus = $state<QwenStatus | null>(null);
  let moliStatus = $state<MoliStatus | null>(null);

  let layaForm = $state<LayaSettings>({ serverExe: '', modelDir: '', port: 10230, threads: 8 });
  let qwenForm = $state<QwenSettings>({ serverExe: '', args: '', port: 17230 });
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
  let logBox = $state<HTMLDivElement | null>(null);
  let pinnedToBottom = true;

  $effect(() => {
    void harness.logs.length;
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
      ? '启动中…（模型载入可能较慢）'
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
                disabled={harness.starting ||
                  !harness.environment?.node ||
                  !harness.environment?.dshInstalled ||
                  !harness.environment?.dshVersionMatches}
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
        {#if harness.environment && !harness.environment.node}
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
        {#if harness.environment && !harness.environment.dshInstalled}
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

      <!-- QWen3.5 -->
      <div class="qx-card p-3.5">
        <div class="flex h-7 items-center justify-between">
          <span class="flex items-center gap-2 text-sm font-medium">
            <span class="size-1.5 rounded-full {dot(qwenTone)}" aria-hidden="true"></span>
            QWen3.5
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
                disabled={qwenBusy !== '' || !qwenForm.serverExe}
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
              <span class="text-xs text-muted">可执行文件</span>
              <input
                class="qx-input mt-0.5 w-full font-mono text-xs"
                bind:value={qwenForm.serverExe}
                placeholder="如 D:\AI\llama\llama-b…\llama-server.exe"
              />
            </label>
            <label class="block">
              <span class="text-xs text-muted">启动参数（缺 --port 时自动追加配置端口）</span>
              <input
                class="qx-input mt-0.5 w-full font-mono text-xs"
                bind:value={qwenForm.args}
                placeholder="-m Qwen3.5-9B-Q6_K.gguf -ngl 99 -c 65536"
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
        <span class="font-mono tabular-nums text-muted/70">{harness.logs.length}</span>
      </div>
      <div
        bind:this={logBox}
        class="min-h-0 flex-1 overflow-y-auto p-3 font-mono text-xs leading-5"
        onscroll={() => {
          if (!logBox) return;
          pinnedToBottom = logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 24;
        }}
      >
        {#if harness.logs.length === 0}
          <p class="text-muted/70">暂无日志</p>
        {:else}
          {#each harness.logs as line, index (index)}
            <div class="whitespace-pre-wrap break-all">{line}</div>
          {/each}
        {/if}
      </div>
    </div>
  </div>
</section>
