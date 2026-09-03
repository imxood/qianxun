<script lang="ts">
  import { onMount } from 'svelte';
  import { harness } from '../../stores/harness.svelte';
  import { formatHarnessStatus } from '../../lib/ipc/contract';

  onMount(() => {
    void harness.backfillLogs();
  });

  let actionError = $state('');
  // 日志容器；状态/日志变化时贴底滚动。
  let logBox = $state<HTMLDivElement | null>(null);
  let pinnedToBottom = true;

  $effect(() => {
    void harness.logs.length;
    if (logBox && pinnedToBottom) logBox.scrollTop = logBox.scrollHeight;
  });

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

  const statusTone: Record<string, string> = {
    stopped: 'text-muted',
    starting: 'text-accent',
    ready: 'text-ok',
    restarting: 'text-accent',
    failed: 'text-danger',
  };
</script>

<section class="mx-auto flex h-full max-w-3xl flex-col gap-4">
  <header class="flex items-center justify-between">
    <div>
      <h1 class="text-lg font-semibold">控制台</h1>
      <p class="mt-1 text-sm {statusTone[harness.status.phase] ?? 'text-muted'}">
        {formatHarnessStatus(harness.status)}
      </p>
    </div>
    <div class="flex gap-2">
      <button
        class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
        disabled={harness.busy || harness.starting}
        onclick={() => void start()}
      >
        启动
      </button>
      <button
        class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft disabled:opacity-50"
        disabled={harness.status.phase === 'stopped' || harness.status.phase === 'failed'}
        onclick={() => void stop()}
      >
        停止
      </button>
    </div>
  </header>

  {#if actionError}
    <p class="rounded-md border border-danger/40 bg-danger/10 p-3 text-sm text-danger">
      {actionError}
    </p>
  {/if}

  <div
    bind:this={logBox}
    class="min-h-0 flex-1 overflow-y-auto rounded-lg border border-line bg-surface p-3 font-mono text-xs leading-5"
    onscroll={() => {
      if (!logBox) return;
      pinnedToBottom = logBox.scrollHeight - logBox.scrollTop - logBox.clientHeight < 24;
    }}
  >
    {#if harness.logs.length === 0}
      <p class="text-muted">暂无输出。DSH 的启动日志、npm 安装进度与健康事件都会流到这里。</p>
    {:else}
      {#each harness.logs as line, index (index)}
        <div class="whitespace-pre-wrap break-all">{line}</div>
      {/each}
    {/if}
  </div>
</section>
