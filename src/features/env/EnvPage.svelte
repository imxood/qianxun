<script lang="ts">
  import { onMount } from 'svelte';
  import { harness } from '../../stores/harness.svelte';
  import { formatHarnessStatus, formatNodeVersion } from '../../lib/ipc/contract';

  onMount(() => {
    void harness.refreshEnvironment();
    // 晚开页不空白：回填缓冲的启动/安装日志。
    void harness.backfillLogs();
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

  /** 进度事件按阶段渲染；百分比总大小未知时转为不定态进度条。 */
  const progress = $derived(harness.installProgress);

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

  function formatBytes(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  function percentOf(downloaded: number, total: number | null): number | null {
    if (!total) return null;
    return Math.min(100, Math.floor((downloaded / total) * 100));
  }

  /**
   * 从 `installSpec`（形如 `@deepseek-ai/dsh@0.1.5-rc.1`）提取精确版本号，
   * 用于「千寻要求 X，当前 Y」展示。比后端再多发一个字段省事——格式稳定。
   */
  function pinnedDshVersion(spec: string): string {
    const at = spec.lastIndexOf('@');
    return at >= 0 ? spec.slice(at + 1) : spec;
  }

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
    <h1 class="text-lg font-semibold">环境</h1>
    <p class="mt-1 text-sm text-muted">安装与启停；过程与结果都在右侧日志。</p>
  </header>

  <div class="flex min-h-0 flex-1 gap-4">
    <!-- 左：操作列 -->
    <aside class="flex w-88 shrink-0 flex-col gap-4 overflow-y-auto pr-1">
      {#if harness.installing && progress}
        <div class="rounded-lg border border-line bg-surface p-4" role="status">
          {#if progress.stage === 'node-download'}
            {@const percent = percentOf(progress.downloadedBytes, progress.totalBytes)}
            <div class="flex items-baseline justify-between gap-3">
              <h2 class="text-sm font-medium">
                下载 Node v{harness.environment?.bundledNodeVersion ?? ''}
              </h2>
              <span class="shrink-0 text-xs text-muted">来源 {progress.source}</span>
            </div>
            <p class="mt-1 truncate font-mono text-xs text-muted" title={progress.url}>
              {progress.url}
            </p>
            <div class="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-line">
              {#if percent === null}
                <div class="h-full w-1/3 animate-pulse rounded-full bg-accent"></div>
              {:else}
                <div
                  class="h-full rounded-full bg-accent transition-[width] duration-500"
                  style={`width:${percent}%`}
                ></div>
              {/if}
            </div>
            <p class="mt-1.5 text-xs text-muted">
              {formatBytes(progress.downloadedBytes)}
              {#if progress.totalBytes}
                <span> / {formatBytes(progress.totalBytes)} · {percent}%</span>
              {/if}
            </p>
          {:else if progress.stage === 'node-manifest'}
            <h2 class="text-sm font-medium">
              获取校验清单<span class="ml-2 text-xs font-normal text-muted">{progress.source}</span>
            </h2>
          {:else if progress.stage === 'node-finalize'}
            <h2 class="text-sm font-medium">
              {progress.activity}<span class="ml-2 text-xs font-normal text-muted"
                >{progress.source}</span
              >
            </h2>
          {:else if progress.stage === 'dsh-packages'}
            {@const percent = percentOf(progress.downloaded, progress.totalHint)}
            <div class="flex items-baseline justify-between gap-3">
              <h2 class="text-sm font-medium">安装 DSH</h2>
              {#if percent !== null}
                <span class="shrink-0 text-xs text-muted">{percent}%</span>
              {/if}
            </div>
            <p class="mt-1 truncate font-mono text-xs text-muted" title={progress.registry}>
              {progress.registry}
            </p>
            <div class="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-line">
              {#if percent === null}
                <div class="h-full w-1/3 animate-pulse rounded-full bg-accent"></div>
              {:else}
                <div
                  class="h-full rounded-full bg-accent transition-[width] duration-500"
                  style={`width:${percent}%`}
                ></div>
              {/if}
            </div>
            <p class="mt-1.5 text-xs text-muted">
              已下载 {progress.downloaded}{#if progress.totalHint}
                / {progress.totalHint}
              {/if}
              个包 · 已安装 {progress.added}
            </p>
          {/if}
        </div>
      {/if}

      <!-- 启动控制 -->
      <div class="rounded-lg border border-line bg-surface p-4">
        <p class="text-sm {statusTone[harness.status.phase] ?? 'text-muted'}">
          {formatHarnessStatus(harness.status)}
        </p>
        <div class="mt-3 flex gap-2">
          {#if running}
            <button
              class="flex-1 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:cursor-not-allowed disabled:opacity-50"
              disabled={harness.restarting}
              data-testid="harness-restart"
              onclick={() => void restart()}
            >
              {harness.restarting ? '重启中…' : '重启 DSH'}
            </button>
            <button
              class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft disabled:opacity-50"
              disabled={harness.restarting || harness.status.phase === 'starting'}
              onclick={() => void stop()}
            >
              停止
            </button>
          {:else}
            <button
              class="flex-1 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
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
          <div
            class="flex items-center justify-between gap-3 rounded-lg border border-line bg-surface p-4"
          >
            <p class="text-sm">
              未检测到 Node
              <span class="ml-2 text-xs text-muted">
                需 v{formatNodeVersion(harness.environment.minimumNode)} 或更高
              </span>
            </p>
            <button
              class="shrink-0 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
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
          <div
            class="flex items-center justify-between gap-3 rounded-lg border border-line bg-surface p-4"
          >
            <p class="text-sm">DSH 未安装</p>
            <button
              class="shrink-0 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
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
            class="flex items-center justify-between gap-3 rounded-lg border border-warning/40 bg-warning/10 p-4"
          >
            <div class="min-w-0">
              <p class="text-sm text-warning">⚠ DSH 版本不匹配，启动已禁用</p>
              <p class="mt-0.5 text-xs text-muted">
                千寻要求
                <span class="font-mono">{pinnedDshVersion(harness.environment.installSpec)}</span>
                ，当前 <span class="font-mono">{harness.environment.dshVersion ?? '?'}</span>
              </p>
            </div>
            <button
              class="shrink-0 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
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

        <!-- 环境信息：默认收起，健康时不占版面；点开看版本与路径事实。 -->
        <details class="rounded-lg border border-line bg-surface px-3 py-2 text-xs">
          <summary class="cursor-pointer select-none text-muted">信息</summary>
          <dl class="mt-2 space-y-2">
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
        </details>
      {/if}
    </aside>

    <!-- 右：日志 -->
    <div
      bind:this={logBox}
      class="min-h-0 min-w-0 flex-1 overflow-y-auto rounded-lg border border-line bg-surface p-3 font-mono text-xs leading-5"
      onscroll={() => {
        if (!logBox) return;
        pinnedToBottom = logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 24;
      }}
    >
      {#if harness.logs.length === 0}
        <p class="text-muted">暂无日志</p>
      {:else}
        {#each harness.logs as line, index (index)}
          <div class="whitespace-pre-wrap break-all">{line}</div>
        {/each}
      {/if}
    </div>
  </div>
</section>
