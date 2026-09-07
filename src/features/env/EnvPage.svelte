<script lang="ts">
  import { onMount } from 'svelte';
  import { harness } from '../../stores/harness.svelte';
  import { formatHarnessStatus, formatNodeVersion } from '../../lib/ipc/contract';

  onMount(() => {
    void harness.refreshEnvironment();
    // 晚开页不空白：回填缓冲的启动/安装日志。
    void harness.backfillLogs();
  });

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

  function formatBytes(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  function percentOf(downloaded: number, total: number | null): number | null {
    if (!total) return null;
    return Math.min(100, Math.floor((downloaded / total) * 100));
  }

  const statusTone: Record<string, string> = {
    stopped: 'text-muted',
    starting: 'text-accent',
    ready: 'text-ok',
    restarting: 'text-accent',
    failed: 'text-danger',
  };

  // 日志容器；状态/日志变化时贴底滚动。
  let logBox = $state<HTMLDivElement | null>(null);
  let pinnedToBottom = true;

  $effect(() => {
    void harness.logs.length;
    if (logBox && pinnedToBottom) logBox.scrollTop = logBox.scrollHeight;
  });
</script>

<!-- 竖向工作台：检查 Node → 检查 DSH → 启动/停止 → 日志铺满剩余空间。
     检查区空间不足时自身滚动，日志面板永远可见。 -->
<section class="flex h-full flex-col gap-4 p-6">
  <header class="shrink-0">
    <h1 class="text-lg font-semibold">环境</h1>
    <p class="mt-1 text-sm text-muted">
      千寻运行依赖 Node.js 与 DSH；安装与启动的详细日志始终在下方实时可见。
    </p>
  </header>

  <div class="min-h-0 shrink space-y-4 overflow-y-auto pr-1">
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

    {#if harness.environmentLoading && !harness.environment}
      <p class="text-sm text-muted">检测中…</p>
    {/if}

    {#if harness.environment}
      <!-- 检查 Node -->
      <div class="rounded-lg border border-line bg-surface p-4">
        <div class="flex items-start justify-between gap-4">
          <div class="min-w-0">
            <h2 class="font-medium">
              检查 Node
              {#if harness.environment.node}
                <span class="ml-1 text-xs text-ok">✓ 就绪</span>
              {:else}
                <span class="ml-1 text-xs text-danger">✗ 未检测到</span>
              {/if}
            </h2>
            {#if harness.environment.node}
              <p class="mt-1 text-sm text-fg">
                {formatNodeVersion(harness.environment.node.version)}
                <span class="ml-2 rounded bg-accent-soft px-1.5 py-0.5 text-xs text-muted">
                  {sourceLabels[harness.environment.node.source] ?? harness.environment.node.source}
                </span>
              </p>
              <p
                class="mt-1 truncate font-mono text-xs text-muted"
                title={harness.environment.node.path}
              >
                {harness.environment.node.path}
              </p>
            {:else}
              <p class="mt-1 text-sm text-danger">
                未检测到 Node.js，需要 v{formatNodeVersion(harness.environment.minimumNode)}
                或更高
              </p>
            {/if}
          </div>
          {#if !harness.environment.node}
            <button
              class="shrink-0 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
              disabled={harness.installing}
              onclick={installNode}
            >
              {harness.installing
                ? '正在安装…'
                : `安装 Node v${harness.environment.bundledNodeVersion}`}
            </button>
          {/if}
        </div>
        {#if harness.environment.allNodeRuntimes.length > 1}
          <details class="mt-3 text-xs text-muted">
            <summary class="cursor-pointer select-none">
              发现 {harness.environment.allNodeRuntimes.length} 个运行时（使用最新）
            </summary>
            <ul class="mt-2 space-y-1">
              {#each harness.environment.allNodeRuntimes as runtime (runtime.path)}
                <li class="break-all font-mono">
                  {formatNodeVersion(runtime.version)} ·
                  {sourceLabels[runtime.source] ?? runtime.source} · {runtime.path}
                </li>
              {/each}
            </ul>
          </details>
        {/if}
        {#if nodeError}
          <p class="mt-3 text-sm text-danger">{nodeError}</p>
        {/if}
      </div>

      <!-- 检查 DSH -->
      <div class="rounded-lg border border-line bg-surface p-4">
        <div class="flex items-start justify-between gap-4">
          <div class="min-w-0">
            <h2 class="font-medium">
              检查 DSH
              {#if harness.environment.dshInstalled}
                <span class="ml-1 text-xs text-ok">✓ 已安装</span>
              {:else}
                <span class="ml-1 text-xs text-danger">✗ 未检测到</span>
              {/if}
            </h2>
            {#if harness.environment.dshInstalled}
              <p class="mt-1 text-sm text-fg">
                已安装
                {#if harness.environment.dshVersion}
                  <span class="font-mono">{harness.environment.dshVersion}</span>
                {/if}
                <span class="ml-2 text-xs text-muted"
                  >锁定 {harness.environment.installSpec}（由千寻版本决定）</span
                >
              </p>
              <p
                class="mt-1 truncate font-mono text-xs text-muted"
                title={harness.environment.dshEntry}
              >
                {harness.environment.dshEntry}
              </p>
            {:else}
              <p class="mt-1 text-sm text-danger">未检测到 DSH</p>
              <p class="mt-1 text-xs text-muted">
                安装说明符 <span class="font-mono">{harness.environment.installSpec}</span
                >（由千寻锁定，经 pnpm 装入应用私有目录）。
              </p>
            {/if}
          </div>
          {#if !harness.environment.dshInstalled || harness.environment.dshVersion}
            <button
              class="shrink-0 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
              disabled={harness.installing || !harness.environment.node}
              onclick={installDsh}
            >
              {harness.installing
                ? '正在安装…'
                : harness.environment.dshInstalled
                  ? '重新安装'
                  : '安装 DSH'}
            </button>
          {/if}
        </div>
        {#if !harness.environment.node}
          <p class="mt-3 text-xs text-muted">安装 DSH 前需先安装 Node。</p>
        {/if}
        <p class="mt-3 text-xs text-muted">
          工作目录 <span class="font-mono">{harness.environment.workspace}</span>
          · DSH_HOME <span class="font-mono">{harness.environment.dshHome}</span>
        </p>
        {#if dshError}
          <p class="mt-3 text-sm text-danger">{dshError}</p>
        {/if}
      </div>
    {/if}

    <!-- 启动 / 停止 -->
    <div class="rounded-lg border border-line bg-surface p-4">
      <div class="flex items-center justify-between gap-4">
        <p class="text-sm {statusTone[harness.status.phase] ?? 'text-muted'}">
          {formatHarnessStatus(harness.status)}
        </p>
        <div class="flex shrink-0 gap-2">
          <button
            class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
            disabled={harness.busy || harness.starting}
            onclick={() => void start()}
          >
            启动 DSH
          </button>
          <button
            class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft disabled:opacity-50"
            disabled={harness.status.phase === 'stopped' || harness.status.phase === 'failed'}
            onclick={() => void stop()}
          >
            停止
          </button>
        </div>
      </div>
      {#if actionError}
        <p class="mt-3 text-sm text-danger">{actionError}</p>
      {/if}
    </div>
  </div>

  <!-- 日志：铺满剩余空间；安装/启动的每一行都实时落在这里。 -->
  <div
    bind:this={logBox}
    class="min-h-32 flex-1 overflow-y-auto rounded-lg border border-line bg-surface p-3 font-mono text-xs leading-5"
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
</section>
