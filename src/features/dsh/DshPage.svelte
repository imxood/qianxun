<script lang="ts">
  import { onMount } from 'svelte';
  import { harness } from '../../stores/harness.svelte';
  import { nav } from '../../stores/nav.svelte';

  /** 独立窗口模式：站内跳转（环境页）不可用（主窗布局不在本窗口）。 */
  let { standalone = false }: { standalone?: boolean } = $props();

  onMount(() => {
    void harness.wire();
  });

  // 状态就绪即加载 iframe；重启/断线由 reload 钩子自动恢复。
  const ready = $derived(harness.status.phase === 'ready' && harness.proxyUrl !== null);
  // DSH 0.1.2 起浏览器认证 cookie 为 SameSite=Strict：跨站 iframe
  // （tauri.localhost 内嵌 127.0.0.1）永远不携带，直连必 401。iframe 一律
  // 走回环代理（harness_proxy_url），cookie 由服务端持有，浏览器侧零
  // cookie。代理进程内绑定一次端口不变；DSH revive 由代理热吸收，iframe
  // 无需重载——地址自始至终就是同一个。
  const dshUrl = $derived(harness.proxyUrl ?? '');

  // DSH 已就绪而代理地址还没拿到（wire 与 setup 监听的启动竞态）时补拉。
  $effect(() => {
    if (harness.status.phase === 'ready' && harness.proxyUrl === null) {
      void harness.refreshProxyUrl();
    }
  });

  // 首屏防白屏：iframe 文档加载完成前用主题色浮层盖住。
  let frameLoaded = $state(false);
  $effect(() => {
    void dshUrl;
    frameLoaded = false;
  });

  const statusText: Record<string, string> = {
    stopped: 'DSH 未运行。可在「环境」页启动，或：',
    starting: 'DSH 启动中…首次启动需加载插件，稍慢。',
    restarting: 'DSH 异常退出，正在自动重启…',
    failed: 'DSH 启动失败，详见「环境」页日志。',
  };
  const stoppedText = $derived(
    standalone ? 'DSH 未运行。请先在主窗口的「环境」页启动。' : statusText.stopped,
  );
</script>

{#if harness.status.phase === 'ready' && !ready}
  <div class="flex h-full w-full items-center justify-center bg-bg">
    <div class="max-w-md space-y-3 text-center">
      <p class="text-sm text-muted">
        DSH 已就绪，但回环代理尚未监听成功。请查看「环境」页日志； 也可<button
          class="text-accent underline-offset-2 hover:underline"
          onclick={() => void harness.refreshProxyUrl()}
        >
          重试获取代理地址</button
        >。
      </p>
    </div>
  </div>
{:else if ready}
  <div class="relative h-full w-full bg-bg">
    <iframe
      title="DSH"
      class="h-full w-full border-0"
      src={dshUrl}
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
        <p class="text-sm text-muted">{stoppedText}</p>
      {/if}
      <div class="flex justify-center gap-2 pt-1">
        {#if !standalone && (harness.status.phase === 'stopped' || harness.status.phase === 'failed')}
          <button
            class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
            disabled={harness.starting}
            onclick={() => void harness.start().catch(() => {})}
          >
            {harness.starting ? '启动中…' : '启动 DSH'}
          </button>
        {/if}
        {#if !standalone}
          <button
            class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
            onclick={() => nav.go('env')}
          >
            查看环境与日志
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}
