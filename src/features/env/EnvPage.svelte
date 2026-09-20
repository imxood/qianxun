<script lang="ts">
  import { onMount } from 'svelte';
  import { harness } from '../../stores/harness.svelte';
  import { formatHarnessStatus, formatNodeVersion } from '../../lib/ipc/contract';
  import { pinnedDshVersion } from '../../lib/utils/dsh-version';

  onMount(() => {
    void harness.refreshEnvironment();
    // 晚开页不空白：回填缓冲的启动/安装日志。
    void harness.backfillLogs();
    // 失败态按钮与安全模式徽标的数据源（08 设计 §11.5）。
    void harness.refreshRecovery();
  });

  /** Node 来源标注（信息块用）。 */
  const sourceLabels: Record<string, string> = {
    path: 'PATH',
    nvm: 'nvm',
    fnm: 'fnm',
    volta: 'Volta',
    system: '系统',
    managed: '千寻',
  };

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

  /** 恢复到上次能启动的配置（08 设计 §11.5；后端停机→恢复→重启一次完成）。 */
  async function recoverKnownGood(): Promise<void> {
    actionError = '';
    recoverSummary = '';
    try {
      const removed = await harness.recoverKnownGood();
      recoverSummary =
        removed.length > 0
          ? `已恢复并重启。从装配清单卸下：${removed.join('、')}（文件保留，可在插件页重装）`
          : '已恢复并重启。';
    } catch (error) {
      actionError = error instanceof Error ? error.message : String(error);
    }
  }

  /** 进入最小安全环境（逃生舱；默认 profile 不受影响）。 */
  async function safeMode(): Promise<void> {
    actionError = '';
    try {
      await harness.safeModeStart();
    } catch (error) {
      actionError = error instanceof Error ? error.message : String(error);
    }
  }

  let recoverSummary = $state('');

  const statusTone: Record<string, string> = {
    stopped: 'text-muted',
    starting: 'text-accent',
    ready: 'text-ok',
    restarting: 'text-accent',
    failed: 'text-danger',
  };

  /** 进程是否在跑：跑着时主按钮是「重启 DSH」而非「启动 DSH」。 */
  const running = $derived(
    harness.status.phase === 'starting' ||
      harness.status.phase === 'ready' ||
      harness.status.phase === 'restarting',
  );

  // 日志容器；状态/日志变化时贴底滚动。
  let logBox = $state<HTMLDivElement | null>(null);
  let pinnedToBottom = true;

  $effect(() => {
    void harness.logs.length;
    if (logBox && pinnedToBottom) logBox.scrollTop = logBox.scrollHeight;
  });
</script>

<!-- 左操作右日志：左侧检测/安装/控制，右侧日志铺满全高实时滚动。 -->
<section class="flex h-full flex-col gap-4 p-6">
  <header class="shrink-0">
    <h1 class="qx-page-title">环境</h1>
  </header>

  <div class="flex min-h-0 flex-1 gap-4">
    <!-- 左：操作列 -->
    <aside class="flex w-88 shrink-0 flex-col gap-3 overflow-y-auto pr-1">
      <!-- 启动控制 -->
      <div class="qx-card p-4">
        <p
          class="flex items-center gap-2 text-sm {statusTone[harness.status.phase] ?? 'text-muted'}"
        >
          <span class="size-2 rounded-full bg-current" aria-hidden="true"></span>
          {formatHarnessStatus(harness.status)}
        </p>
        <div class="mt-3 flex gap-2">
          {#if running}
            <button
              class="qx-btn qx-btn-primary qx-btn-md flex-1"
              disabled={harness.restarting}
              data-testid="harness-restart"
              onclick={() => void restart()}
            >
              {harness.restarting ? '重启中…' : '重启 DSH'}
            </button>
            <button
              class="qx-btn qx-btn-outline qx-btn-md"
              disabled={harness.restarting || harness.status.phase === 'starting'}
              onclick={() => void stop()}
            >
              停止
            </button>
          {:else}
            <button
              class="qx-btn qx-btn-primary qx-btn-md flex-1"
              disabled={harness.starting ||
                !harness.environment?.node ||
                !harness.environment?.dshInstalled ||
                !harness.environment?.dshVersionMatches}
              data-testid="harness-start"
              onclick={() => void start()}
            >
              {harness.starting ? '启动中…' : '启动 DSH'}
            </button>
          {/if}
        </div>
        {#if harness.status.phase === 'failed'}
          <p class="mt-2 text-xs text-danger">{harness.status.reason}</p>
          <!-- 三级自救 UI（08 设计 §11.5）：到这里说明启动链（含快照自动
               恢复重试）已失败。快照在 → 手动恢复按钮；安全模式兜底。 -->
          <div class="mt-3 flex flex-col gap-2">
            {#if harness.recovery.snapshotAvailable}
              <button
                class="qx-btn qx-btn-outline qx-btn-md w-full"
                disabled={harness.recovering}
                data-testid="harness-recover"
                onclick={() => void recoverKnownGood()}
              >
                {harness.recovering ? '恢复中…' : '恢复到上次能启动的配置'}
              </button>
            {/if}
            <button
              class="qx-btn qx-btn-outline qx-btn-md w-full"
              disabled={harness.recovering}
              data-testid="harness-safe-mode"
              onclick={() => void safeMode()}
            >
              安全模式启动
            </button>
            <p class="text-muted/80">
              {harness.recovery.snapshotAvailable
                ? '自动恢复已尝试仍失败：可手动重试，或进入只读的安全模式排查插件。'
                : '没有可用快照。安全模式是只读的最小环境，默认 profile 不受影响。'}
            </p>
          </div>
        {/if}
        {#if recoverSummary}
          <p class="mt-2 text-xs text-ok" role="status">{recoverSummary}</p>
        {/if}
        {#if harness.recovery.safeMode && harness.status.phase === 'ready'}
          <!-- 安全模式徽标（08 设计 §11.5）：插件变更被冻结的提示位。 -->
          <p
            class="mt-2 rounded-lg border border-warning/40 bg-warning/10 px-2 py-1.5 text-xs text-warning"
            data-testid="safe-mode-badge"
          >
            安全模式中：插件变更已冻结。「重启 DSH」即返回默认 profile。
          </p>
        {/if}
        {#if actionError}
          <p class="mt-2 text-xs text-danger">{actionError}</p>
        {/if}
      </div>

      {#if harness.environmentLoading && !harness.environment}
        <p class="text-sm text-muted">检测中…</p>
      {/if}

      {#if harness.environment}
        <!-- 只在缺东西时出现：环境健康时这一列为空，日志即环境页的主体。 -->
        {#if !harness.environment.node}
          <div class="qx-card flex items-center justify-between gap-3 p-4">
            <p class="text-sm">
              未检测到 Node
              <span class="ml-2 text-xs text-muted">
                需 v{formatNodeVersion(harness.environment.minimumNode)}+
              </span>
            </p>
            <button
              class="qx-btn qx-btn-primary qx-btn-md"
              disabled={harness.installing}
              onclick={installNode}
            >
              {harness.installing ? '安装中…' : `安装 v${harness.environment.bundledNodeVersion}`}
            </button>
          </div>
          {#if nodeError}
            <p class="text-sm text-danger">{nodeError}</p>
          {/if}
        {/if}

        {#if !harness.environment.dshInstalled}
          <!-- 未安装：给一个安装入口，过程与结果都在右侧日志。 -->
          <div class="qx-card flex items-center justify-between gap-3 p-4">
            <p class="text-sm">DSH 未安装</p>
            <button
              class="qx-btn qx-btn-primary qx-btn-md"
              disabled={harness.installing || !harness.environment.node}
              onclick={installDsh}
            >
              {harness.installing ? '安装中…' : '安装'}
            </button>
          </div>
          {#if dshError}
            <p class="text-sm text-danger">{dshError}</p>
          {/if}
        {:else if !harness.environment.dshVersionMatches}
          <!-- ADR-015：已装但版本对不上时禁止启动，必须升级。 -->
          <div
            class="flex items-center justify-between gap-3 rounded-xl border border-warning/40 bg-warning/10 p-4"
          >
            <div class="min-w-0">
              <p class="text-sm font-medium text-warning">DSH 版本不匹配，启动已禁用</p>
              <p class="mt-0.5 text-xs text-muted">
                要求 <span class="font-mono"
                  >{pinnedDshVersion(harness.environment.installSpec)}</span
                >
                ，当前 <span class="font-mono">{harness.environment.dshVersion ?? '?'}</span>
              </p>
            </div>
            <button
              class="qx-btn qx-btn-primary qx-btn-md"
              disabled={harness.installing || !harness.environment.node}
              onclick={installDsh}
            >
              {harness.installing ? '安装中…' : '升级'}
            </button>
          </div>
          {#if dshError}
            <p class="text-sm text-danger">{dshError}</p>
          {/if}
        {/if}

        <!-- 环境信息：常驻展示版本与路径事实。 -->
        <div class="qx-card overflow-hidden text-xs">
          <div class="flex h-8 shrink-0 items-center border-b border-line/70 px-3">
            <span class="font-medium text-muted">信息</span>
          </div>
          <dl class="space-y-2 px-3 py-2.5">
            <div class="flex gap-2">
              <dt class="w-16 shrink-0 text-muted">DSH</dt>
              <dd class="min-w-0 flex-1 font-mono">
                <span>
                  {harness.environment.dshVersion ??
                    `未安装（要求 ${pinnedDshVersion(harness.environment.installSpec)}）`}
                </span>
                <p class="truncate text-muted" title={harness.environment.dshEntry}>
                  {harness.environment.dshEntry}
                </p>
              </dd>
            </div>
            {#if harness.environment.node}
              <div class="flex gap-2">
                <dt class="w-16 shrink-0 text-muted">Node</dt>
                <dd class="min-w-0 flex-1 font-mono">
                  {formatNodeVersion(harness.environment.node.version)}
                  <span class="text-muted">
                    · {sourceLabels[harness.environment.node.source] ??
                      harness.environment.node.source}
                  </span>
                  <p class="truncate text-muted" title={harness.environment.node.path}>
                    {harness.environment.node.path}
                  </p>
                </dd>
              </div>
            {/if}
            <div class="flex gap-2">
              <dt class="w-16 shrink-0 text-muted">工作目录</dt>
              <dd class="min-w-0 flex-1 truncate font-mono" title={harness.environment.workspace}>
                {harness.environment.workspace}
              </dd>
            </div>
            <div class="flex gap-2">
              <dt class="w-16 shrink-0 text-muted">DSH_HOME</dt>
              <dd class="min-w-0 flex-1 truncate font-mono" title={harness.environment.dshHome}>
                {harness.environment.dshHome}
              </dd>
            </div>
          </dl>
        </div>
      {/if}
    </aside>

    <!-- 右：日志 -->
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
