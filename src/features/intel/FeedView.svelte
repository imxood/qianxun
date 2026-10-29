<script lang="ts">
  import { call } from '../../lib/ipc';
  import type { Article } from '../../lib/ipc/contract';
  import { intel, sourceLabel } from '../../stores/intel.svelte';
  import { formatTime } from './format';

  /**
   * Feed 主题视图（docs/17 §7.4）：默认显示 Agent 出刊的文章流，
   * 「原始源」tab 保留未加工条目列表；文章点击进阅读视图。
   */

  async function open(itemUrl: string, itemId: string): Promise<void> {
    intel.markRead([itemId]);
    try {
      await call('window_open_external', { url: itemUrl, newWindow: false });
    } catch {
      /* 打开失败不打断阅读 */
    }
  }

  const sortedArticles = $derived(
    [...(intel.articles?.articles ?? [])].sort((a, b) => b.importance - a.importance),
  );

  function openArticle(article: Article): void {
    intel.reading = article;
  }
</script>

<header class="flex shrink-0 items-center gap-2 px-6 pb-3 pt-5">
  <button
    class="qx-btn qx-btn-sm qx-btn-ghost"
    onclick={() => intel.back()}
    data-testid="intel-back"
  >
    ‹ 情报
  </button>
  <h1 class="qx-page-title">{intel.topic(intel.active)?.label ?? ''}</h1>
  <span
    class="ml-1 flex items-center rounded-lg bg-surface p-0.5"
    role="tablist"
    aria-label="视图切换"
  >
    <button
      class="qx-segment {intel.feedTab === 'digest' ? 'qx-segment-on' : 'qx-segment-off'}"
      role="tab"
      aria-selected={intel.feedTab === 'digest'}
      onclick={() => (intel.feedTab = 'digest')}
      data-testid="intel-tab-digest"
    >
      文章
    </button>
    <button
      class="qx-segment {intel.feedTab === 'raw' ? 'qx-segment-on' : 'qx-segment-off'}"
      role="tab"
      aria-selected={intel.feedTab === 'raw'}
      onclick={() => (intel.feedTab = 'raw')}
      data-testid="intel-tab-raw"
    >
      原始源
    </button>
  </span>
  {#if intel.feedTab === 'raw'}
    <span class="ml-2 flex items-center gap-1" aria-label="日期导航">
      <button
        class="qx-icon-btn"
        onclick={() => void intel.goOlder()}
        disabled={!intel.canGoOlder}
        aria-label="前一天"
        data-testid="intel-date-older"
      >
        <svg
          viewBox="0 0 24 24"
          class="size-4"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M15 5l-7 7 7 7" />
        </svg>
      </button>
      <span class="min-w-16 text-center font-mono text-xs text-muted">
        {intel.date || '今天'}
      </span>
      <button
        class="qx-icon-btn"
        onclick={() => void intel.goNewer()}
        disabled={!intel.canGoNewer}
        aria-label="后一天"
        data-testid="intel-date-newer"
      >
        <svg
          viewBox="0 0 24 24"
          class="size-4"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M9 5l7 7-7 7" />
        </svg>
      </button>
    </span>
    {#if intel.date}
      <button class="qx-btn qx-btn-sm qx-btn-ghost" onclick={() => void intel.changeDate('')}>
        回到今天
      </button>
    {/if}
  {/if}
  <div class="flex-1"></div>
  {#if intel.feedTab === 'digest'}
    <button
      class="qx-btn qx-btn-sm qx-btn-outline"
      onclick={() => void intel.startDigest()}
      disabled={intel.digestRunning}
      data-testid="intel-digest-start"
    >
      {intel.digestRunning ? '出刊中…' : '重新出刊'}
    </button>
  {/if}
  <button
    class="qx-btn qx-btn-sm qx-btn-outline"
    onclick={() => void intel.refresh()}
    disabled={intel.refreshing || intel.topic(intel.active)?.fetching}
    data-testid="intel-refresh"
  >
    {intel.refreshing || intel.topic(intel.active)?.fetching ? '抓取中…' : '刷新'}
  </button>
</header>

<!-- 出刊进行中横幅（点击打开状态页） -->
{#if intel.digestRunning}
  <button
    class="mx-6 mb-2 flex shrink-0 items-center gap-2 rounded-lg bg-accent-soft/50 px-3 py-1.5 text-left text-xs text-accent"
    onclick={() => void intel.openStatus()}
    data-testid="intel-digest-banner"
  >
    <span class="size-1.5 animate-pulse rounded-full bg-accent"></span>
    <span class="truncate">{intel.digest?.message ?? '出刊进行中…'}</span>
    <span class="ml-auto shrink-0 font-mono">{intel.digest?.done}/{intel.digest?.total}</span>
  </button>
{/if}

<div class="min-h-0 flex-1 overflow-y-auto px-6 pb-6" aria-live="polite">
  {#if intel.feedTab === 'raw'}
    <!-- 原始源：未加工条目列表（原文标题永不改写） -->
    {#if intel.loading}
      <div class="space-y-2">
        {#each Array(6) as _, i (i)}
          <div class="qx-card h-16 animate-pulse opacity-40"></div>
        {/each}
      </div>
    {:else if !intel.day || intel.day.items === null || intel.day.items.length === 0}
      <div class="flex h-full flex-col items-center justify-center gap-3 text-muted">
        <p class="text-sm" data-testid="intel-empty-today">
          {intel.date ? '无记录' : '今日未抓'}
        </p>
        {#if !intel.date}
          <button class="qx-btn qx-btn-sm qx-btn-primary" onclick={() => void intel.refresh()}>
            抓取
          </button>
        {/if}
      </div>
    {:else}
      <div class="qx-card divide-y divide-line/60">
        {#each intel.day.items as item (item.id)}
          {@const read = intel.day?.read[item.id] !== undefined}
          {@const saved = intel.saved(item.id)}
          <article class="flex items-start gap-3 px-4 py-3">
            <div class="min-w-0 flex-1">
              <button
                class="block w-full truncate text-left text-sm font-medium transition-colors hover:text-accent {read
                  ? 'text-muted'
                  : ''}"
                title={item.title}
                onclick={() => void open(item.url, item.id)}
                data-testid="intel-feed-title"
              >
                {item.title}
              </button>
              <div class="mt-1 flex items-center gap-2 text-xs text-muted">
                <span class="qx-badge bg-accent-soft/60 text-accent"
                  >{sourceLabel(item.sourceId)}</span
                >
                {#if item.heat !== null}
                  <span class="font-mono">▲ {item.heat}</span>
                {/if}
                <span>{formatTime(item.publishedAt)}</span>
              </div>
              {#if item.summary}
                <p class="mt-1 line-clamp-2 text-xs leading-relaxed text-muted">{item.summary}</p>
              {/if}
            </div>
            <button
              class="qx-icon-btn mt-0.5"
              aria-label={saved ? '取消收藏' : '收藏'}
              aria-pressed={saved}
              onclick={() => void intel.toggleSaveFeed(item)}
              data-testid={'intel-feed-star-' + item.id}
            >
              <svg
                viewBox="0 0 24 24"
                class="size-4"
                fill={saved ? 'currentColor' : 'none'}
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
                stroke-linejoin="round"
                class:text-accent={saved}
                aria-hidden="true"
              >
                <path d="M12 17.3l-6.2 3.6 1.7-7L2 9.2l7.1-.6L12 2l2.9 6.6 7.1.6-5.5 4.7 1.7 7z" />
              </svg>
            </button>
          </article>
        {/each}
      </div>
    {/if}
  {:else if intel.reading}
    <!-- 阅读视图 -->
    <article class="mx-auto max-w-3xl" data-testid="intel-article-view">
      <button class="qx-btn qx-btn-sm qx-btn-ghost -ml-3" onclick={() => (intel.reading = null)}>
        ‹ 返回列表
      </button>
      <div class="mt-1 flex flex-wrap items-center gap-2">
        {#each Array(intel.reading.importance) as _, i (i)}
          <span class="text-xs text-warning" aria-hidden="true">★</span>
        {/each}
        {#each intel.reading.tags as tag (tag)}
          <span class="qx-badge bg-accent-soft/60 text-accent">{tag}</span>
        {/each}
      </div>
      <h2 class="mt-2 text-2xl leading-snug font-semibold">{intel.reading.title}</h2>
      {#if intel.reading.lead}
        <p class="mt-3 text-base leading-relaxed text-muted">{intel.reading.lead}</p>
      {/if}
      <div class="mt-5 space-y-4">
        {#each intel.reading.body as paragraph, i (i)}
          <p class="text-[15px] leading-7">{paragraph}</p>
        {/each}
      </div>
      <footer class="mt-8 border-t border-line pt-4">
        <h3 class="text-xs font-medium text-muted">来源</h3>
        <ul class="mt-2 space-y-1">
          {#each intel.reading.sources as source (source.itemId)}
            <li>
              <button
                class="flex w-full items-center gap-2 text-left text-xs text-accent hover:underline"
                title={source.title}
                onclick={() => void open(source.url, source.itemId)}
              >
                <span class="qx-badge bg-accent-soft/60 text-accent"
                  >{sourceLabel(source.sourceId)}</span
                >
                <span class="truncate">{source.title}</span>
              </button>
            </li>
          {/each}
        </ul>
        <p class="mt-3 text-xs text-muted">
          由 {intel.reading.model} 加工 · {formatTime(intel.reading.createdAt)}
        </p>
      </footer>
    </article>
  {:else if sortedArticles.length > 0}
    <!-- 出刊文章流（头条大卡 + 紧凑卡，docs/17 §7.5.2-4/5） -->
    <div class="mx-auto max-w-3xl">
      {#if intel.articlesFallback}
        <div
          class="mb-3 rounded-lg bg-surface px-3 py-1.5 text-xs text-muted"
          data-testid="intel-fallback-banner"
        >
          显示 {intel.articles?.date} 刊 · 今日出刊完成后自动更新
        </div>
      {/if}
      <div class="space-y-3">
        {#each sortedArticles as article, index (article.id)}
          {@const saved = intel.saved(article.id)}
          {@const headline = index === 0 && article.importance >= 5}
          <article
            class="qx-card transition-colors hover:border-accent/40 {headline
              ? 'border-accent/30 p-6'
              : 'p-5'}"
          >
            <div class="flex items-start justify-between gap-3">
              <button
                class="min-w-0 flex-1 text-left"
                onclick={() => openArticle(article)}
                data-testid="intel-article-card"
              >
                <div class="flex flex-wrap items-center gap-1.5">
                  {#if headline}
                    <span class="qx-badge bg-accent-soft text-accent">头条</span>
                  {/if}
                  {#each Array(article.importance) as _, i (i)}
                    <span class="text-xs text-warning" aria-hidden="true">★</span>
                  {/each}
                  {#each article.tags.slice(0, 3) as tag (tag)}
                    <span class="qx-badge bg-accent-soft/60 text-accent">{tag}</span>
                  {/each}
                  {#if article.sources.length > 1}
                    <span class="qx-badge bg-surface text-muted">{article.sources.length} 源</span>
                  {/if}
                </div>
                <h2 class="mt-1.5 leading-snug font-semibold {headline ? 'text-xl' : 'text-lg'}">
                  {article.title}
                </h2>
                {#if article.lead}
                  <p
                    class="mt-1.5 text-sm leading-relaxed text-muted {headline
                      ? ''
                      : 'line-clamp-2'}"
                  >
                    {article.lead}
                  </p>
                {/if}
              </button>
              <button
                class="qx-icon-btn mt-1 shrink-0"
                aria-label={saved ? '取消收藏' : '收藏'}
                aria-pressed={saved}
                onclick={() => void intel.toggleSaveArticle(article)}
                data-testid={'intel-article-star-' + article.id}
              >
                <svg
                  viewBox="0 0 24 24"
                  class="size-4"
                  fill={saved ? 'currentColor' : 'none'}
                  stroke="currentColor"
                  stroke-width="1.6"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  class:text-accent={saved}
                  aria-hidden="true"
                >
                  <path
                    d="M12 17.3l-6.2 3.6 1.7-7L2 9.2l7.1-.6L12 2l2.9 6.6 7.1.6-5.5 4.7 1.7 7z"
                  />
                </svg>
              </button>
            </div>
          </article>
        {/each}
      </div>
    </div>
  {:else if intel.digestRunning}
    <div class="flex h-full items-center justify-center">
      <p class="text-sm text-muted">Agent 编辑部加工中，请看右下角进度…</p>
    </div>
  {:else if intel.digest?.stage === 'llm-unavailable'}
    <div class="flex h-full flex-col items-center justify-center gap-3">
      <p class="text-sm text-danger" data-testid="intel-llm-unavailable">{intel.digest.message}</p>
      <p class="text-xs text-muted">DSH 启动后点「重新出刊」即可生成今日中文文章</p>
    </div>
  {:else if intel.loading}
    <div class="space-y-2">
      {#each Array(4) as _, i (i)}
        <div class="qx-card h-24 animate-pulse opacity-40"></div>
      {/each}
    </div>
  {:else}
    <div class="flex h-full flex-col items-center justify-center gap-3 text-muted">
      <p class="text-sm" data-testid="intel-empty-digest">
        {intel.date ? '该日未出刊' : '今日尚未出刊'}
      </p>
      {#if !intel.date}
        <button class="qx-btn qx-btn-sm qx-btn-primary" onclick={() => void intel.startDigest()}>
          开始出刊
        </button>
      {/if}
    </div>
  {/if}
</div>
