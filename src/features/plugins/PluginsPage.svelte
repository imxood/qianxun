<script lang="ts">
  /**
   * 插件市场（发现 / 已安装）。
   *
   * 插件 = 装进 DSH profile 的 npm 包。发现 = registry 搜索（默认 npmmirror，
   * 空词给生态发现词）；安装 = pnpm 精确版本 + 并入 profile bundles。
   * 变更即时落盘、DSH 下次启动生效；pnpm 输出进 supervisor 日志，页面底部
   * 跟一行最新日志当进度。
   */
  import { onMount } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { call } from '../../lib/ipc';
  import type { MarketDetail, MarketInstalled, MarketListing } from '../../lib/ipc/contract';
  import { harness } from '../../stores/harness.svelte';

  type Tab = 'discover' | 'installed';

  const DEBOUNCE_MS = 320;

  let tab = $state<Tab>('discover');
  let query = $state('');
  let results = $state<MarketListing[]>([]);
  let searching = $state(false);
  let searchSeq = 0;
  let expanded = $state<string | null>(null);
  let details = new SvelteMap<string, MarketDetail>();
  let installed = $state<MarketInstalled[]>([]);
  let installedLoading = $state(false);
  let working = $state<string | null>(null);
  let error = $state('');

  onMount(() => {
    void refreshInstalled();
  });

  // 搜索：空词立即（首屏发现），输入防抖。
  $effect(() => {
    const keyword = query;
    const current = tab;
    if (current === 'installed') return;
    const delay = keyword.trim() === '' ? 0 : DEBOUNCE_MS;
    const timer = window.setTimeout(() => void search(keyword), delay);
    return () => window.clearTimeout(timer);
  });

  async function search(keyword: string): Promise<void> {
    const seq = ++searchSeq;
    searching = true;
    error = '';
    try {
      const items = await call<MarketListing[]>('market_search', { query: keyword });
      if (seq === searchSeq) results = items;
    } catch (failure) {
      if (seq === searchSeq) {
        results = [];
        error = failure instanceof Error ? failure.message : String(failure);
      }
    } finally {
      if (seq === searchSeq) searching = false;
    }
  }

  async function refreshInstalled(): Promise<void> {
    installedLoading = true;
    try {
      installed = await call<MarketInstalled[]>('market_installed');
    } catch {
      installed = [];
    } finally {
      installedLoading = false;
    }
  }

  async function toggleExpand(name: string): Promise<void> {
    error = '';
    expanded = expanded === name ? null : name;
    if (expanded !== name) return;
    await loadDetail(name);
  }

  async function loadDetail(name: string): Promise<void> {
    if (details.has(name)) return;
    try {
      details.set(name, await call<MarketDetail>('market_detail', { name }));
    } catch (failure) {
      error = failure instanceof Error ? failure.message : String(failure);
      expanded = null;
    }
  }

  async function install(name: string, version: string): Promise<void> {
    working = name;
    error = '';
    try {
      await loadDetail(name);
      await call('market_install', { name, version });
      await refreshInstalled();
    } catch (failure) {
      error = failure instanceof Error ? failure.message : String(failure);
    } finally {
      working = null;
    }
  }

  async function remove(name: string): Promise<void> {
    working = name;
    error = '';
    try {
      await call('market_remove', { name });
      await refreshInstalled();
    } catch (failure) {
      error = failure instanceof Error ? failure.message : String(failure);
    } finally {
      working = null;
    }
  }

  function installedHere(name: string): MarketInstalled | undefined {
    return installed.find((entry) => entry.name === name);
  }

  // 展开行里安装按钮要精确版本：详情没有时退回列表给的版本。
  function versionOf(listing: MarketListing): string {
    return details.get(listing.name)?.version ?? listing.version;
  }

  function compatText(detail: MarketDetail): string {
    switch (detail.compatibility.state) {
      case 'compatible':
        return `兼容 DSH（${detail.compatibility.requirement}）`;
      case 'incompatible':
        return `不兼容：${detail.compatibility.reason}`;
      default:
        return '未声明 DSH 版本';
    }
  }

  function count(value: number): string {
    if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
    if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
    return String(value);
  }

  function day(iso: string): string {
    return iso.slice(0, 10);
  }

  function filesize(bytes: number | null): string {
    if (bytes === null) return '';
    if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  // 安装/卸载走 pnpm，输出进 supervisor 日志；尾行即进度。
  const progressLine = $derived(harness.logs.at(-1) ?? '');
</script>

<section class="flex h-full min-h-0 flex-col">
  <!-- 头：标题 + tab + 搜索 -->
  <header class="shrink-0 space-y-3 border-b border-line px-6 pt-5 pb-3">
    <div class="flex items-center justify-between gap-4">
      <h1 class="text-lg font-semibold">插件</h1>
      <div class="flex rounded-lg border border-line p-0.5 text-sm">
        <button
          class="rounded-md px-3 py-1 transition-colors {tab === 'discover'
            ? 'bg-accent-soft font-medium text-fg'
            : 'text-muted hover:text-fg'}"
          onclick={() => (tab = 'discover')}
        >
          发现
        </button>
        <button
          class="rounded-md px-3 py-1 transition-colors {tab === 'installed'
            ? 'bg-accent-soft font-medium text-fg'
            : 'text-muted hover:text-fg'}"
          onclick={() => (tab = 'installed')}
        >
          已安装 {installed.length > 0 ? installed.length : ''}
        </button>
      </div>
    </div>
    {#if tab === 'discover'}
      <input
        class="w-full rounded-md border border-line bg-surface px-3 py-1.5 text-sm outline-none focus:border-accent"
        type="search"
        placeholder="搜索 npm 上的 DSH 插件"
        bind:value={query}
      />
    {/if}
  </header>

  {#if tab === 'discover' && error}
    <p class="shrink-0 px-6 pt-3 text-sm text-danger">{error}</p>
  {/if}

  <!-- 列表 -->
  <div class="min-h-0 flex-1 overflow-y-auto px-6 py-3">
    {#if tab === 'discover'}
      {#if results.length === 0}
        <p class="py-12 text-center text-sm text-muted">
          {searching ? '搜索中…' : '没有结果'}
        </p>
      {:else}
        <ul>
          {#each results as listing (listing.name)}
            {@const here = installedHere(listing.name)}
            {@const detail = details.get(listing.name)}
            <li class="border-b border-line/60 last:border-b-0">
              <div class="flex items-start gap-3 py-3">
                <button
                  class="min-w-0 flex-1 text-left"
                  onclick={() => void toggleExpand(listing.name)}
                >
                  <p class="flex items-baseline gap-2">
                    <span class="truncate text-sm font-medium">{listing.name}</span>
                    <span class="shrink-0 font-mono text-xs text-muted">{listing.version}</span>
                    {#if here}
                      <span class="shrink-0 text-xs text-ok">已安装</span>
                    {/if}
                  </p>
                  {#if listing.description}
                    <p class="mt-1 line-clamp-2 text-xs leading-relaxed text-muted">
                      {listing.description}
                    </p>
                  {/if}
                  <p class="mt-1 flex flex-wrap gap-x-3 text-xs text-muted">
                    {#if listing.publisher}<span>{listing.publisher}</span>{/if}
                    {#if listing.weeklyDownloads > 0}
                      <span>{count(listing.weeklyDownloads)} 周下载</span>
                    {/if}
                    {#if listing.updated}<span>{day(listing.updated)}</span>{/if}
                  </p>
                </button>
                {#if here}
                  <span class="shrink-0 px-1 py-1 text-xs text-ok">✓</span>
                {:else}
                  <button
                    class="shrink-0 rounded-md border border-line px-2.5 py-1 text-xs font-medium transition-colors hover:bg-accent-soft disabled:opacity-40"
                    disabled={working !== null}
                    onclick={() => void install(listing.name, versionOf(listing))}
                  >
                    {working === listing.name ? '安装中…' : '安装'}
                  </button>
                {/if}
              </div>
              {#if expanded === listing.name}
                <div class="mb-3 space-y-1.5 rounded-md border border-line bg-surface p-3 text-xs">
                  {#if !detail}
                    <p class="text-muted">读取详情…</p>
                  {:else}
                    <p class="flex flex-wrap items-center gap-2">
                      <span
                        class="rounded px-1.5 py-0.5 {detail.bundle
                          ? 'bg-ok/15 text-ok'
                          : 'bg-accent-soft text-muted'}"
                      >
                        {detail.bundle ? 'DSH 插件' : '普通包'}
                      </span>
                      <span class="text-muted">{compatText(detail)}</span>
                      {#if detail.license}<span class="text-muted">{detail.license}</span>{/if}
                      {#if detail.unpackedBytes}
                        <span class="text-muted">{filesize(detail.unpackedBytes)}</span>
                      {/if}
                    </p>
                    {#if detail.deprecated}
                      <p class="text-danger">已弃用：{detail.deprecated}</p>
                    {/if}
                    {#if detail.lifecycleScripts.length > 0}
                      <p class="text-muted">
                        含安装脚本：{detail.lifecycleScripts.join('、')}
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
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    {:else if installed.length === 0}
      <p class="py-12 text-center text-sm text-muted">
        {installedLoading ? '读取中…' : '尚未安装插件'}
      </p>
    {:else}
      <ul>
        {#each installed as entry (entry.name)}
          <li class="flex items-center gap-3 border-b border-line/60 py-2.5 last:border-b-0">
            <div class="min-w-0 flex-1">
              <p class="flex items-baseline gap-2">
                <span class="truncate text-sm">{entry.name}</span>
                {#if entry.spec}
                  <span class="shrink-0 font-mono text-xs text-muted">{entry.spec}</span>
                {/if}
              </p>
              <p class="mt-0.5 flex items-center gap-2 text-xs">
                {#if entry.builtin}
                  <span class="rounded bg-accent-soft px-1.5 py-0.5 text-muted">内核</span>
                {:else if entry.active}
                  <span class="rounded bg-ok/15 px-1.5 py-0.5 text-ok">生效中</span>
                {:else}
                  <span class="rounded bg-accent-soft px-1.5 py-0.5 text-muted">未加载</span>
                {/if}
                {#if !entry.deployed}
                  <span class="text-danger">文件缺失，重装可修复</span>
                {/if}
              </p>
            </div>
            {#if !entry.builtin}
              <button
                class="shrink-0 rounded-md border border-line px-2.5 py-1 text-xs text-danger transition-colors hover:bg-danger/10 disabled:opacity-40"
                disabled={working !== null}
                onclick={() => void remove(entry.name)}
              >
                {working === entry.name ? '卸载中…' : '卸载'}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- 底：进度行 + 生效提示 -->
  <footer class="flex h-8 shrink-0 items-center gap-2 border-t border-line px-6">
    {#if working !== null}
      <span class="size-2 animate-pulse rounded-full bg-accent"></span>
      <span class="truncate font-mono text-xs text-muted">{progressLine}</span>
    {:else}
      <span class="text-xs text-muted">安装 / 卸载在 DSH 下次启动时生效</span>
    {/if}
  </footer>
</section>
