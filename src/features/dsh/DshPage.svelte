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

  const phase = $derived(harness.status.phase);
  /** failed 阶段的失败原因（其余阶段为空串；同时为 TS 收窄联合类型）。 */
  const failReason = $derived(harness.status.phase === 'failed' ? harness.status.reason : '');
</script>

{#if phase === 'ready' && !ready}
  <div class="flex h-full w-full items-center justify-center bg-bg">
    <div class="max-w-md space-y-3 text-center">
      <p class="text-sm text-muted">
        DSH 已就绪，但回环代理尚未监听。请查看「环境」页日志，或
        <button
          class="text-accent underline-offset-2 hover:underline"
          onclick={() => void harness.refreshProxyUrl()}
        >
          重试获取代理地址
        </button>
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
    <div class="flex max-w-md flex-col items-center gap-4 text-center">
      {#if phase === 'failed'}
        <span
          class="grid size-12 place-items-center rounded-2xl bg-danger/10 text-danger"
          aria-hidden="true"
        >
          <svg
            viewBox="0 0 24 24"
            class="size-6"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
          >
            <path
              d="M12 8v5M12 16.5v.5M10.3 3.8L2.8 17a2 2 0 001.7 3h15a2 2 0 001.7-3L13.7 3.8a2 2 0 00-3.4 0z"
            />
          </svg>
        </span>
        <p class="text-sm font-medium text-fg">DSH 启动失败</p>
        <p
          class="max-w-md w-full rounded-xl border border-line bg-card p-3 text-left font-mono text-xs break-all text-muted"
        >
          {failReason}
        </p>
      {:else if phase === 'starting' || phase === 'restarting'}
        <span
          class="inline-block size-8 animate-spin rounded-full border-2 border-line border-t-accent"
          aria-hidden="true"
        ></span>
        <p class="text-sm text-muted">{phase === 'starting' ? 'DSH 启动中…' : 'DSH 重启中…'}</p>
      {:else}
        <!-- e2e 依赖「DSH 未运行」文案。 -->
        <span
          class="qx-grad grid size-14 place-items-center rounded-2xl text-white shadow-lg shadow-accent/30"
          aria-hidden="true"
        >
          <svg
            viewBox="0 0 24 24"
            class="size-7"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 3l8 4.5v9L12 21l-8-4.5v-9zM12 12l8-4.5M12 12v9M12 12L4 7.5" />
          </svg>
        </span>
        <p class="text-sm text-muted">
          {standalone ? 'DSH 未运行。请先在主窗口的「环境」页启动。' : 'DSH 未运行'}
        </p>
      {/if}
      <div class="flex justify-center gap-2">
        {#if !standalone && (phase === 'stopped' || phase === 'failed')}
          <button
            class="qx-btn qx-btn-primary qx-btn-md"
            disabled={harness.starting}
            onclick={() => void harness.start().catch(() => {})}
          >
            {harness.starting ? '启动中…' : '启动 DSH'}
          </button>
        {/if}
        {#if !standalone}
          <button class="qx-btn qx-btn-outline qx-btn-md" onclick={() => nav.go('env')}>
            环境与日志
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}
