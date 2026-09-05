<script lang="ts">
  import { onMount } from 'svelte';
  import { harness } from '../../stores/harness.svelte';
  import { nav } from '../../stores/nav.svelte';

  onMount(() => {
    void harness.wire();
  });

  // 状态就绪即加载 iframe；重启/断线由 reload 钩子自动恢复。
  const ready = $derived(harness.status.phase === 'ready');
  const origin = $derived(harness.status.phase === 'ready' ? harness.status.origin : '');
  const launchToken = $derived(harness.status.phase === 'ready' ? harness.status.token : '');
  // DSH 0.1.2 起 index 需要启动 token（换取签名 cookie）；旧版 token 为空
  // 串时退回裸 origin。重启后 origin 不变，iframe 不会自动重载：origin 每次变化
  // 就换一个无害查询参数强制刷新。effect 只读 origin、只写 frameSrc——写自己
  // 读不到的状态才不会自环（reloadToken++ 的先读后写曾把 effect 变成死循环，
  // 未捕获异常卡死整个调度器，表现为「进了 DSH 页就点不动任何页面」）。
  let reloadSeq = 0;
  let frameSrc = $state('');
  $effect(() => {
    reloadSeq += 1;
    const base = origin ? `${origin}/` : '';
    const query = launchToken
      ? `token=${encodeURIComponent(launchToken)}&qx=${reloadSeq}`
      : `qx=${reloadSeq}`;
    frameSrc = base ? `${base}?${query}` : '';
  });

  // 首屏防白屏：iframe 文档加载完成前用主题色浮层盖住。配合外壳的后台
  // 预热（DSH 就绪即挂载本页），正常点进来时浮层早已消失，只剩兜底作用。
  let frameLoaded = $state(false);
  $effect(() => {
    void frameSrc;
    frameLoaded = false;
  });

  const statusText: Record<string, string> = {
    stopped: 'DSH 未运行。可在「环境」页启动，或：',
    starting: 'DSH 启动中…首次启动需加载插件，稍慢。',
    restarting: 'DSH 异常退出，正在自动重启…',
    failed: 'DSH 启动失败，详见「环境」页日志。',
  };
</script>

{#if ready}
  <div class="relative h-full w-full bg-bg">
    <iframe
      title="DSH"
      class="h-full w-full border-0"
      src={frameSrc}
      sandbox="allow-scripts allow-same-origin allow-forms allow-downloads allow-popups"
      onload={() => (frameLoaded = true)}
    ></iframe>
    {#if !frameLoaded}
      <div class="absolute inset-0 z-10 flex items-center justify-center bg-bg">
        <div class="flex items-center gap-2 text-sm text-muted">
          <span
            class="inline-block size-3.5 animate-spin rounded-full border-2 border-line border-t-accent"
            aria-hidden="true"
          ></span>
          正在加载 DSH…
        </div>
      </div>
    {/if}
  </div>
{:else}
  <div class="flex h-full w-full items-center justify-center bg-bg">
    <div class="max-w-md space-y-3 text-center">
      {#if harness.status.phase === 'failed'}
        <p class="text-sm text-danger">{statusText.failed}</p>
        <p class="break-all rounded-md bg-surface p-3 text-left font-mono text-xs text-muted">
          {harness.status.reason}
        </p>
      {:else if harness.status.phase === 'starting' || harness.status.phase === 'restarting'}
        <p class="text-sm text-muted">{statusText[harness.status.phase]}</p>
      {:else}
        <p class="text-sm text-muted">{statusText.stopped}</p>
      {/if}
      <div class="flex justify-center gap-2 pt-1">
        {#if harness.status.phase === 'stopped' || harness.status.phase === 'failed'}
          <button
            class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
            disabled={harness.starting}
            onclick={() => void harness.start().catch(() => {})}
          >
            {harness.starting ? '启动中…' : '启动 DSH'}
          </button>
        {/if}
        <button
          class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
          onclick={() => nav.go('env')}
        >
          查看环境与日志
        </button>
      </div>
    </div>
  </div>
{/if}
