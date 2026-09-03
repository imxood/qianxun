<script lang="ts">
  import { onMount } from 'svelte';
  import { harness } from '../../stores/harness.svelte';
  import { formatNodeVersion } from '../../lib/ipc/contract';

  onMount(() => {
    void harness.refreshEnvironment();
  });

  const sourceLabels: Record<string, string> = {
    path: 'PATH',
    nvm: 'nvm',
    fnm: 'fnm',
    volta: 'Volta',
    system: '系统安装',
    managed: '千寻下载',
  };

  let installError = $state('');

  async function installDsh(): Promise<void> {
    installError = '';
    try {
      await harness.install();
    } catch (error) {
      installError = error instanceof Error ? error.message : String(error);
    }
  }

  async function installNode(): Promise<void> {
    installError = '';
    try {
      await harness.installNode();
    } catch (error) {
      installError = error instanceof Error ? error.message : String(error);
    }
  }
</script>

<section class="mx-auto max-w-2xl space-y-6">
  <header>
    <h1 class="text-lg font-semibold">环境</h1>
    <p class="mt-1 text-sm text-muted">
      千寻需要 Node.js 与 DSH。检测不到的组件会在这里给出安装入口；安装落在本应用
      数据目录，不污染系统环境。
    </p>
  </header>

  {#if harness.environmentLoading && !harness.environment}
    <p class="text-sm text-muted">探测中…</p>
  {/if}

  {#if harness.environment}
    <!-- Node -->
    <div class="rounded-lg border border-line bg-surface p-4">
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="font-medium">Node.js</h2>
          {#if harness.environment.node}
            <p class="mt-1 text-sm text-fg">
              {formatNodeVersion(harness.environment.node.version)}
              <span class="ml-2 rounded bg-accent-soft px-1.5 py-0.5 text-xs text-muted">
                {sourceLabels[harness.environment.node.source] ?? harness.environment.node.source}
              </span>
            </p>
            <p class="mt-1 break-all font-mono text-xs text-muted">
              {harness.environment.node.path}
            </p>
          {:else}
            <p class="mt-1 text-sm text-danger">
              未检测到可用的 Node.js（需 v{formatNodeVersion(harness.environment.minimumNode)}
              或更高）
            </p>
            <p class="mt-1 text-xs text-muted">
              一键安装会下载官方 win-x64 zip（镜像源按设置，auto =官方优先失败转 npmmirror）、校验
              SHA-256 后解压到千寻数据目录，不改动系统 PATH。
            </p>
            <button
              class="mt-3 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
              disabled={harness.installing}
              onclick={() => void installNode()}
            >
              {harness.installing
                ? '安装中…（下载数十 MB，进度见控制台）'
                : '一键安装 Node v22.19.0'}
            </button>
          {/if}
        </div>
      </div>
      {#if harness.environment.allNodeRuntimes.length > 1}
        <details class="mt-3 text-xs text-muted">
          <summary class="cursor-pointer select-none">
            发现 {harness.environment.allNodeRuntimes.length} 个运行时（已选最新）
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
    </div>

    <!-- DSH -->
    <div class="rounded-lg border border-line bg-surface p-4">
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0">
          <h2 class="font-medium">DSH</h2>
          {#if harness.environment.dshInstalled}
            <p class="mt-1 text-sm text-fg">
              已安装
              {#if harness.environment.dshVersion}
                <span class="font-mono">{harness.environment.dshVersion}</span>
              {/if}
            </p>
            <p class="mt-1 break-all font-mono text-xs text-muted">
              {harness.environment.dshEntry}
            </p>
          {:else}
            <p class="mt-1 text-sm text-danger">未检测到 DSH</p>
            <p class="mt-1 text-xs text-muted">
              安装源走 npm（默认 npmmirror 淘宝源，可在设置里更改），装进千寻私有目录。
            </p>
          {/if}
        </div>
        {#if !harness.environment.dshInstalled || harness.environment.dshVersion}
          <button
            class="shrink-0 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
            disabled={harness.installing || !harness.environment.node}
            onclick={() => void installDsh()}
          >
            {harness.installing
              ? '安装中…'
              : harness.environment.dshInstalled
                ? '重新安装'
                : '安装 DSH'}
          </button>
        {/if}
      </div>
      {#if !harness.environment.node}
        <p class="mt-3 text-xs text-muted">需要先有 Node 才能安装 DSH。</p>
      {/if}
      {#if installError}
        <p class="mt-3 text-sm text-danger">{installError}</p>
      {/if}
    </div>

    <!-- 布局信息 -->
    <div class="rounded-lg border border-line bg-surface p-4 text-xs text-muted">
      <p>
        工作目录：<span class="font-mono">{harness.environment.workspace}</span>
      </p>
      <p class="mt-1">
        DSH_HOME：<span class="font-mono">{harness.environment.dshHome}</span>
        <span class="ml-2">（与系统 ~/.dsh 隔离，见设置）</span>
      </p>
    </div>
  {/if}
</section>
