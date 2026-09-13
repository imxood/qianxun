<script lang="ts">
  /**
   * 插件详情对话框（模态）。
   *
   * 兼容性等信息需要单独请求，放对话框而不是行内展开——覆盖层不改变
   * 列表布局，高度零跳动。Esc / 点遮罩关闭；安装动作经 oninstall 回到
   * 页面（Rust 安装预检仍是最终门槛）。
   */
  import { compatBadge, count, filesize } from '../../lib/market/format';
  import type { MarketDetail, MarketListing } from '../../lib/market/types';

  let {
    listing,
    detail,
    loading,
    error,
    installing,
    installed,
    blockReason,
    oninstall,
    onclose,
  }: {
    listing: MarketListing | null;
    detail: MarketDetail | null;
    loading: boolean;
    error: string;
    installing: boolean;
    installed: boolean;
    /** 非空 = 不可安装的原因（不兼容/已弃用）。 */
    blockReason: string | null;
    oninstall: () => void;
    onclose: () => void;
  } = $props();

  function onKeydown(event: KeyboardEvent): void {
    if (!listing) return;
    if (event.key === 'Escape') onclose();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if listing}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6"
    role="presentation"
    onclick={onclose}
  >
    <div
      class="flex max-h-[80vh] w-full max-w-xl flex-col rounded-xl border border-line bg-surface shadow-xl"
      role="dialog"
      aria-modal="true"
      aria-label={listing.name}
      onclick={(event) => event.stopPropagation()}
    >
      <!-- 头 -->
      <div class="flex items-start justify-between gap-3 border-b border-line p-4">
        <div class="min-w-0">
          <p class="flex flex-wrap items-baseline gap-2">
            <span class="truncate text-sm font-semibold">{listing.name}</span>
            <span class="shrink-0 font-mono text-xs text-muted">
              {detail?.version ?? listing.version}
            </span>
            {#if installed}
              <span class="rounded bg-ok/15 px-1.5 py-0.5 text-xs text-ok">已安装</span>
            {/if}
          </p>
          {#if listing.publisher}
            <p class="mt-0.5 text-xs text-muted">{listing.publisher}</p>
          {/if}
        </div>
        <button class="qx-icon-btn" aria-label="关闭" onclick={onclose}> ✕ </button>
      </div>

      <!-- 正文 -->
      <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-4 text-xs leading-relaxed">
        {#if loading && !detail}
          <p class="text-muted">读取详情…</p>
        {:else if error && !detail}
          <p class="text-danger">{error}</p>
        {:else if detail}
          {@const compat = compatBadge(detail)}
          <p class="flex flex-wrap items-center gap-2">
            <span
              class="rounded px-1.5 py-0.5 {detail.bundle
                ? 'bg-ok/15 text-ok'
                : 'bg-accent-soft text-muted'}"
            >
              {detail.bundle ? 'DSH 插件' : '普通包'}
            </span>
            <span class="rounded px-1.5 py-0.5 {compat.cls}">{compat.text}</span>
            {#if detail.license}<span class="text-muted">{detail.license}</span>{/if}
            {#if detail.unpackedBytes}
              <span class="text-muted">{filesize(detail.unpackedBytes)}</span>
            {/if}
          </p>
          {#if detail.deprecated}
            <p class="text-danger">已弃用：{detail.deprecated}</p>
          {/if}
          {#if detail.lifecycleScripts.length > 0}
            <p class="text-muted">含安装脚本：{detail.lifecycleScripts.join('、')}</p>
          {/if}
          {#if detail.description}
            <p class="text-muted">{detail.description}</p>
          {/if}
          {#if listing.weeklyDownloads > 0 || (listing.stars !== null && listing.stars > 0)}
            <p class="flex flex-wrap gap-x-3 text-muted">
              {#if listing.weeklyDownloads > 0}
                <span>{count(listing.weeklyDownloads)} 周下载</span>
              {/if}
              {#if listing.stars !== null && listing.stars > 0}
                <span>★ {count(listing.stars)}</span>
              {/if}
            </p>
          {/if}
          {#if detail.homepage || detail.repository}
            <p class="truncate">
              <span class="text-muted">主页 </span>
              <span class="font-mono">{detail.homepage ?? detail.repository}</span>
            </p>
          {/if}
          <p class="font-mono text-muted">将安装 {detail.installSpec}</p>
        {/if}
      </div>

      <!-- 底 -->
      <div class="flex items-center justify-end gap-2 border-t border-line p-4">
        {#if error}<span class="min-w-0 flex-1 truncate text-xs text-danger">{error}</span>{/if}
        <button class="qx-btn qx-btn-ghost rounded-md px-3 py-1.5 text-xs" onclick={onclose}>
          关闭
        </button>
        {#if !installed}
          <button
            class="qx-btn rounded-md px-3 py-1.5 text-xs font-medium text-white {blockReason
              ? 'qx-btn-ghost opacity-50'
              : 'qx-btn-primary'} disabled:opacity-40"
            disabled={installing || blockReason !== null}
            title={blockReason ?? ''}
            onclick={oninstall}
          >
            {installing ? '安装中…' : blockReason !== null ? `不可安装（${blockReason}）` : '安装'}
          </button>
        {/if}
      </div>
    </div>
  </div>
{/if}
