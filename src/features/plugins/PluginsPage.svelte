<script lang="ts">
  /**
   * 插件市场：推荐（catalog）/ 搜索（npm）/ 已安装。
   *
   * 推荐源 = dsh-plugin-catalog（awesome-dsh-plugin 每日构建，npm 包承载，
   * 跟随镜像设置）；搜索 = registry search 端点（去中心化兜底）。浏览数据
   * 全部在 webview 里 fetch（系统代理自动生效），tar.gz 解包零依赖。
   *
   * 兼容性等深度信息需要逐条请求：放详情对话框（覆盖层，列表布局零跳动），
   * 点击时单独请求；「仅看兼容 DSH」筛选开启时才对当前页按需检查（4 并发）。
   * 安装/卸载仍走 Rust（market_install 内部有 registry 复核硬门槛）。
   * 变更即时落盘、DSH 下次启动生效；pnpm 输出进 supervisor 日志，底部
   * 跟一行最新日志当进度。
   */
  import { onMount } from 'svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { call } from '../../lib/ipc';
  import type { MarketInstalled } from '../../lib/ipc/contract';
  import { loadCatalog } from '../../lib/market/catalog';
  import { fetchNpmDetail, registryBase, searchNpm } from '../../lib/market/npm';
  import { compatBadge, count, filesize } from '../../lib/market/format';
  import type { CatalogData, MarketDetail, MarketListing } from '../../lib/market/types';
  import { pinnedDshVersion } from '../../lib/utils/dsh-version';
  import { harness } from '../../stores/harness.svelte';
  import { settings } from '../../stores/settings.svelte';
  import MarketDetailDialog from './MarketDetailDialog.svelte';

  type Tab = 'featured' | 'search' | 'installed';
  /** 推荐排序键：目录顺序 = 上游策展原序（类内按收录时间倒序）。 */
  type FeaturedSort = 'downloads' | 'added' | 'stars' | 'catalog';
  type SearchSort = 'relevance' | 'updated' | 'downloads' | 'name';

  const DEBOUNCE_MS = 320;
  const PAGE_SIZE = 100;
  const DETAIL_CONCURRENCY = 4;

  let tab = $state<Tab>('featured');
  let featuredSort = $state<FeaturedSort>('downloads');
  let searchSort = $state<SearchSort>('relevance');

  /** registry base（镜像设置；变化时缓存按 key 失效重拉）。 */
  const registry = $derived(registryBase(settings.current?.mirrors.npmRegistry));
  /** 千寻钉住的 DSH 精确版本（兼容展示用）。 */
  const pinned = $derived(pinnedDshVersion(harness.environment?.installSpec ?? ''));

  let installed = $state<MarketInstalled[]>([]);
  let installedLoading = $state(false);
  let working = $state<string | null>(null);
  let error = $state('');

  // ---- 推荐 ----
  let catalog = $state<CatalogData | null>(null);
  let catalogLoading = $state(false);
  let catalogError = $state('');
  let featuredFilter = $state('');
  let featuredCategory = $state<string | null>(null);
  let featuredLimit = $state(PAGE_SIZE);

  // ---- 搜索 ----
  let query = $state('');
  let searchResults = $state<MarketListing[]>([]);
  let searching = $state(false);
  let searchSeq = 0;
  let searchError = $state('');

  // ---- 兼容性详情（点击请求；「DSH版本」筛选非全部时才按需批量检查） ----
  // null = 读取失败，缺失 = 未请求。key 带 registry + pinned，切换自动失效。
  let details = new SvelteMap<string, MarketDetail | null>();
  const inflight = new SvelteSet<string>();
  /**
   * DSH 版本筛选：all = 全部；compatible = 兼容当前钉住的 DSH；
   * none = 未声明 DSH 版本；`req:<范围>` = 声明了该范围（兼容与否看徽标）。
   */
  type DshFilter = 'all' | 'compatible' | 'none' | `req:${string}`;
  let dshFilter = $state<DshFilter>('all');

  function detailKey(name: string): string {
    return `${registry}\0${pinned}\0${name}`;
  }

  // ---- 详情对话框 ----
  let selected = $state<MarketListing | null>(null);
  let dialogDetail = $state<MarketDetail | null>(null);
  let dialogLoading = $state(false);
  let dialogError = $state('');

  onMount(() => {
    void refreshInstalled();
  });

  // 目录：进入页面拉一次；registry 变化时缓存 key 失效自动重拉。
  $effect(() => {
    void registry;
    void loadFeatured(false);
  });

  // 搜索：空词立即（生态发现），输入防抖；只在搜索 tab。
  $effect(() => {
    const keyword = query;
    if (tab !== 'search') return;
    const delay = keyword.trim() === '' ? 0 : DEBOUNCE_MS;
    const timer = window.setTimeout(() => void runSearch(keyword), delay);
    return () => window.clearTimeout(timer);
  });

  // 「DSH版本」筛选非全部时才对当前页做兼容性检查（4 并发）；全部 = 零请求。
  $effect(() => {
    if (dshFilter === 'all') return;
    const scope = tab === 'featured' ? featuredBase.slice(0, featuredLimit) : searchBase;
    void ensureDetails(scope);
  });

  async function loadFeatured(force: boolean): Promise<void> {
    catalogLoading = true;
    catalogError = '';
    try {
      catalog = await loadCatalog(registry, force);
    } catch (failure) {
      catalog = null;
      catalogError = failure instanceof Error ? failure.message : String(failure);
    } finally {
      catalogLoading = false;
    }
  }

  async function runSearch(keyword: string): Promise<void> {
    const seq = ++searchSeq;
    searching = true;
    searchError = '';
    try {
      const items = await searchNpm(registry, keyword);
      if (seq === searchSeq) searchResults = items;
    } catch (failure) {
      if (seq === searchSeq) {
        searchResults = [];
        searchError = failure instanceof Error ? failure.message : String(failure);
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

  /** 按需检查一组条目（4 并发；单条失败记 null，展示「未知」）。 */
  async function ensureDetails(listings: MarketListing[]): Promise<void> {
    const pending = listings.filter(
      (listing) => !details.has(detailKey(listing.name)) && !inflight.has(detailKey(listing.name)),
    );
    if (pending.length === 0) return;
    for (const listing of pending) inflight.add(detailKey(listing.name));
    let index = 0;
    const workers = Array.from(
      { length: Math.min(DETAIL_CONCURRENCY, pending.length) },
      async () => {
        while (index < pending.length) {
          const listing = pending[index];
          index += 1;
          if (!listing) break;
          try {
            details.set(
              detailKey(listing.name),
              await fetchNpmDetail(registry, listing.name, pinned),
            );
          } catch {
            details.set(detailKey(listing.name), null);
          } finally {
            inflight.delete(detailKey(listing.name));
          }
        }
      },
    );
    await Promise.all(workers);
  }

  /** 是否禁止安装：明确不兼容或已弃用（Rust 安装预检还会再拦一道）。 */
  function installBlocked(name: string): string | null {
    const detail = details.get(detailKey(name));
    if (detail === null || detail === undefined) return null;
    if (detail.deprecated) return '已弃用';
    if (detail.compatibility.state === 'incompatible') return '与当前 DSH 不兼容';
    return null;
  }

  /** 打开详情对话框：请求一次该条的 registry manifest（缓存命中即直接展示）。 */
  async function openDetail(listing: MarketListing): Promise<void> {
    error = '';
    selected = listing;
    dialogDetail = details.get(detailKey(listing.name)) ?? null;
    if (dialogDetail) {
      dialogLoading = false;
      dialogError = '';
      return;
    }
    dialogLoading = true;
    dialogError = '';
    try {
      const detail = await fetchNpmDetail(registry, listing.name, pinned);
      details.set(detailKey(listing.name), detail);
      dialogDetail = detail;
    } catch (failure) {
      dialogError = failure instanceof Error ? failure.message : String(failure);
    } finally {
      dialogLoading = false;
    }
  }

  function closeDetail(): void {
    selected = null;
    dialogError = '';
  }

  async function install(name: string, version: string): Promise<void> {
    working = name;
    error = '';
    try {
      const target = details.get(detailKey(name))?.version ?? version;
      await call('market_install', { name, version: target });
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

  /** 从系统浏览器打开插件主页（GitHub 等）。 */
  function httpsLink(link: string): boolean {
    return /^https?:\/\//iu.test(link);
  }

  async function openListingLink(listing: MarketListing): Promise<void> {
    if (!listing.link) return;
    error = '';
    try {
      await openUrl(listing.link);
    } catch (cause) {
      error = `打开链接失败：${cause instanceof Error ? cause.message : String(cause)}`;
    }
  }

  function installedHere(name: string): MarketInstalled | undefined {
    return installed.find((entry) => entry.name === name);
  }

  // 推荐基础列表：分类 + 关键词本地过滤 + 排序（不含兼容性筛选）。
  const featuredBase = $derived.by(() => {
    if (!catalog) return [];
    const keyword = featuredFilter.trim().toLowerCase();
    const filtered = catalog.entries.filter((listing) => {
      if (featuredCategory !== null && !listing.categoryIds.includes(featuredCategory)) {
        return false;
      }
      if (keyword === '') return true;
      return (
        listing.name.toLowerCase().includes(keyword) ||
        listing.description.toLowerCase().includes(keyword) ||
        listing.publisher.toLowerCase().includes(keyword)
      );
    });
    switch (featuredSort) {
      case 'downloads':
        filtered.sort((a, b) => b.weeklyDownloads - a.weeklyDownloads);
        break;
      case 'stars':
        filtered.sort((a, b) => (b.stars ?? 0) - (a.stars ?? 0));
        break;
      case 'added':
        // 收录日期（YYYY-MM-DD）倒序；空值沉底。
        filtered.sort((a, b) => b.updated.localeCompare(a.updated));
        break;
      case 'catalog':
        break; // 上游策展原序。
    }
    return filtered;
  });

  /** 单条是否命中当前 DSH 版本筛选（未检查的条目在筛选激活时不显示）。 */
  function matchDshFilter(listing: MarketListing): boolean {
    if (dshFilter === 'all') return true;
    const detail = details.get(detailKey(listing.name));
    if (!detail) return false;
    switch (dshFilter) {
      case 'compatible':
        return detail.compatibility.state === 'compatible';
      case 'none':
        return detail.compatibility.state === 'unknown';
      default: {
        // `req:<范围>`：声明了该范围即命中（兼容与否行内徽标已说明）。
        const requirement = dshFilter.slice(4);
        return (
          detail.compatibility.state !== 'unknown' &&
          detail.compatibility.requirement === requirement
        );
      }
    }
  }

  // DSH 版本筛选作用在基础列表上：未检查的条目在筛选激活时暂不显示。
  const featuredShown = $derived(featuredBase.filter((listing) => matchDshFilter(listing)));

  const searchBase = $derived(searchResults);
  const searchShown = $derived(searchBase.filter((listing) => matchDshFilter(listing)));

  /** 版本范围列表：当前列表里已检查条目声明过的 `@deepseek-ai/dsh` 范围。 */
  const dshRequirements = $derived.by(() => {
    const base = tab === 'featured' ? featuredBase : searchBase;
    const requirements: string[] = [];
    for (const listing of base) {
      const detail = details.get(detailKey(listing.name));
      if (!detail || detail.compatibility.state === 'unknown') continue;
      const requirement = detail.compatibility.requirement;
      if (!requirements.includes(requirement)) requirements.push(requirement);
    }
    return requirements.sort();
  });

  /** 检查进度（筛选激活且未完成时提示）。 */
  const checkProgress = $derived.by(() => {
    if (dshFilter === 'all') return null;
    const scope = tab === 'featured' ? featuredBase.slice(0, featuredLimit) : searchBase;
    const done = scope.filter((listing) => details.has(detailKey(listing.name))).length;
    return { done, total: scope.length, finished: done >= scope.length };
  });

  // 安装/卸载走 pnpm，输出进 supervisor 日志；尾行即进度。
  const progressLine = $derived(harness.logs.at(-1) ?? '');
</script>

{#snippet listingRow(listing: MarketListing)}
  {@const here = installedHere(listing.name)}
  {@const cached = details.get(detailKey(listing.name))}
  <li class="border-b border-line/60 py-3 last:border-b-0">
    <div class="flex items-start gap-3">
      <!-- 主体可点：打开详情对话框（覆盖层，不引起列表高度变化）。 -->
      <button
        class="min-w-0 flex-1 text-left"
        title="查看详情"
        onclick={() => void openDetail(listing)}
      >
        <p class="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <span class="truncate text-sm font-medium">{listing.name}</span>
          <span class="shrink-0 font-mono text-xs text-muted">
            {cached?.version ?? listing.version}
          </span>
          {#if here}
            <span class="shrink-0 rounded bg-ok/15 px-1.5 py-0.5 text-xs text-ok">已安装</span>
          {/if}
          {#if cached}
            <span
              class="shrink-0 rounded px-1.5 py-0.5 text-xs {cached.bundle
                ? 'bg-ok/15 text-ok'
                : 'bg-accent-soft text-muted'}"
            >
              {cached.bundle ? 'DSH 插件' : '普通包'}
            </span>
            {@const compat = compatBadge(cached)}
            <span class="shrink-0 rounded px-1.5 py-0.5 text-xs {compat.cls}">
              {compat.text}
            </span>
            {#if cached.deprecated}
              <span class="shrink-0 rounded bg-danger/10 px-1.5 py-0.5 text-xs text-danger">
                已弃用
              </span>
            {/if}
          {/if}
        </p>
        {#if listing.description}
          <p class="mt-1 line-clamp-2 text-xs leading-relaxed text-muted">
            {listing.description}
          </p>
        {/if}
        <p class="mt-1 flex flex-wrap gap-x-3 text-xs text-muted">
          {#if listing.publisher}<span>{listing.publisher}</span>{/if}
          {#if listing.stars !== null && listing.stars > 0}
            <span>★ {count(listing.stars)}</span>
          {/if}
          {#if listing.weeklyDownloads > 0}
            <span>{count(listing.weeklyDownloads)} 周下载</span>
          {/if}
          {#if listing.updated}<span>{listing.updated}</span>{/if}
          {#if cached?.license}<span>{cached.license}</span>{/if}
          {#if cached?.unpackedBytes}
            <span>{filesize(cached.unpackedBytes)}</span>
          {/if}
        </p>
      </button>
      <div class="flex shrink-0 gap-2">
        {#if listing.link && httpsLink(listing.link)}
          <button
            class="rounded-md border border-line px-2.5 py-1 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
            title={listing.link}
            onclick={() => void openListingLink(listing)}
          >
            打开主页
          </button>
        {/if}
        {#if here}
          <span class="px-1 py-1 text-xs text-ok">✓</span>
        {:else}
          <button
            class="rounded-md border border-line px-2.5 py-1 text-xs font-medium transition-colors hover:bg-accent-soft disabled:opacity-40"
            disabled={working !== null}
            onclick={() => void install(listing.name, listing.version)}
          >
            {working === listing.name ? '安装中…' : '安装'}
          </button>
        {/if}
      </div>
    </div>
  </li>
{/snippet}

<section class="flex h-full min-h-0 flex-col">
  <!-- 头：标题 + tab -->
  <header class="shrink-0 border-b border-line px-6 pt-5 pb-3">
    <div class="flex items-center justify-between gap-4">
      <h1 class="text-lg font-semibold">插件</h1>
      <div class="flex rounded-lg border border-line p-0.5 text-sm">
        {#each [['featured', '推荐'], ['search', '搜索'], ['installed', '已安装']] as [value, label] (value)}
          <button
            class="rounded-md px-3 py-1 transition-colors {tab === value
              ? 'bg-accent-soft font-medium text-fg'
              : 'text-muted hover:text-fg'}"
            onclick={() => (tab = value as Tab)}
          >
            {label}{#if value === 'installed' && installed.length > 0}&nbsp;{installed.length}{/if}
          </button>
        {/each}
      </div>
    </div>
  </header>

  <!-- 推荐 / 搜索的工具行 -->
  {#if tab === 'featured'}
    <div class="flex shrink-0 flex-wrap items-center gap-2 border-b border-line px-6 py-2.5">
      <input
        class="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-1.5 text-sm outline-none focus:border-accent"
        type="search"
        placeholder="过滤推荐插件"
        bind:value={featuredFilter}
        oninput={() => (featuredLimit = PAGE_SIZE)}
      />
      {#if catalog}
        <span class="text-xs text-muted">目录 {catalog.updated}</span>
      {/if}
      <label class="flex shrink-0 items-center gap-1.5 text-xs text-muted">
        排序
        <select class="qx-select py-1 text-xs" bind:value={featuredSort}>
          <option value="downloads">下载数</option>
          <option value="added">最近收录</option>
          <option value="stars">星数</option>
          <option value="catalog">目录顺序</option>
        </select>
      </label>
      <label class="flex shrink-0 items-center gap-1.5 text-xs text-muted">
        DSH版本
        <select class="qx-select max-w-48 py-1 text-xs" bind:value={dshFilter}>
          <option value="all">全部</option>
          <option value="compatible">兼容当前（{pinned}）</option>
          <option value="none">无版本声明</option>
          {#each dshRequirements as requirement (requirement)}
            <option value={`req:${requirement}`}>{requirement}</option>
          {/each}
        </select>
      </label>
      <button
        class="rounded-md border border-line px-2.5 py-1.5 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg disabled:opacity-40"
        disabled={catalogLoading}
        onclick={() => void loadFeatured(true)}
      >
        {catalogLoading ? '刷新中…' : '刷新'}
      </button>
    </div>
    {#if catalog && catalog.categories.length > 0}
      <div class="flex shrink-0 flex-wrap gap-1.5 border-b border-line px-6 py-2">
        <button
          class="rounded-full px-2.5 py-0.5 text-xs transition-colors {featuredCategory === null
            ? 'bg-accent text-white'
            : 'bg-accent-soft text-muted hover:text-fg'}"
          onclick={() => {
            featuredCategory = null;
            featuredLimit = PAGE_SIZE;
          }}
        >
          全部
        </button>
        {#each catalog.categories as category (category.id)}
          <button
            class="rounded-full px-2.5 py-0.5 text-xs transition-colors {featuredCategory ===
            category.id
              ? 'bg-accent text-white'
              : 'bg-accent-soft text-muted hover:text-fg'}"
            onclick={() => {
              featuredCategory = featuredCategory === category.id ? null : category.id;
              featuredLimit = PAGE_SIZE;
            }}
          >
            {category.label}
          </button>
        {/each}
      </div>
    {/if}
  {:else if tab === 'search'}
    <div class="flex shrink-0 flex-wrap items-center gap-2 border-b border-line px-6 py-2.5">
      <input
        class="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-1.5 text-sm outline-none focus:border-accent"
        type="search"
        placeholder="搜索 npm 上的 DSH 插件"
        bind:value={query}
      />
      <label class="flex shrink-0 items-center gap-1.5 text-xs text-muted">
        排序
        <select class="qx-select py-1 text-xs" bind:value={searchSort}>
          <option value="relevance">相关性</option>
          <option value="updated">更新时间</option>
          <option value="downloads">下载数</option>
          <option value="name">名称</option>
        </select>
      </label>
      <label class="flex shrink-0 items-center gap-1.5 text-xs text-muted">
        DSH版本
        <select class="qx-select max-w-48 py-1 text-xs" bind:value={dshFilter}>
          <option value="all">全部</option>
          <option value="compatible">兼容当前（{pinned}）</option>
          <option value="none">无版本声明</option>
          {#each dshRequirements as requirement (requirement)}
            <option value={`req:${requirement}`}>{requirement}</option>
          {/each}
        </select>
      </label>
    </div>
  {/if}

  {#if tab !== 'installed' && error}
    <p class="shrink-0 px-6 pt-3 text-sm text-danger">{error}</p>
  {/if}
  {#if tab === 'featured' && catalogError}
    <p class="shrink-0 px-6 pt-3 text-sm text-danger">{catalogError}</p>
  {/if}
  {#if tab === 'search' && searchError}
    <p class="shrink-0 px-6 pt-3 text-sm text-danger">{searchError}</p>
  {/if}
  {#if checkProgress && !checkProgress.finished}
    <p class="shrink-0 px-6 pt-3 text-xs text-muted">
      正在检查兼容性 {checkProgress.done}/{checkProgress.total}…
    </p>
  {/if}

  <!-- 列表 -->
  <div class="min-h-0 flex-1 overflow-y-auto px-6 py-3">
    {#if tab === 'featured'}
      {#if catalogLoading && !catalog}
        <p class="py-12 text-center text-sm text-muted">加载推荐目录…</p>
      {:else if !catalog}
        <p class="py-12 text-center text-sm text-muted">目录不可用，点「刷新」重试</p>
      {:else if featuredShown.length === 0}
        <p class="py-12 text-center text-sm text-muted">
          {dshFilter !== 'all' ? '没有匹配当前筛选的插件（或仍在检查）' : '没有匹配的插件'}
        </p>
      {:else}
        <ul>
          {#each featuredShown.slice(0, featuredLimit) as listing (listing.name)}
            {@render listingRow(listing)}
          {/each}
        </ul>
        {#if featuredShown.length > featuredLimit}
          <div class="py-4 text-center">
            <button
              class="rounded-md border border-line px-3 py-1.5 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
              onclick={() => (featuredLimit += PAGE_SIZE)}
            >
              加载更多（已显示 {featuredLimit} / {featuredShown.length}）
            </button>
          </div>
        {/if}
      {/if}
    {:else if tab === 'search'}
      {#if searchShown.length === 0}
        <p class="py-12 text-center text-sm text-muted">
          {searching ? '搜索中…' : '没有结果'}
        </p>
      {:else}
        <ul>
          {#each searchShown as listing (listing.name)}
            {@render listingRow(listing)}
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

<MarketDetailDialog
  listing={selected}
  detail={dialogDetail}
  loading={dialogLoading}
  error={dialogError}
  installing={selected !== null && working === selected.name}
  installed={selected !== null && installedHere(selected.name) !== undefined}
  blockReason={selected ? installBlocked(selected.name) : null}
  oninstall={() => {
    if (selected) void install(selected.name, selected.version);
  }}
  onclose={closeDetail}
/>
