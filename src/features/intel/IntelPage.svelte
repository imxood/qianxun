<script lang="ts">
  import { onMount } from 'svelte';
  import { intel } from '../../stores/intel.svelte';
  import { formatTime } from './format';
  import FeedView from './FeedView.svelte';
  import FavoritesView from './FavoritesView.svelte';
  import StatusDialog from './StatusDialog.svelte';
  import FrontPage from './FrontPage.svelte';

  /**
   * 情报页（docs/17）：入口 = 主题卡片网格，点卡片进主题视图。
   * 文案简洁：数据优先，无修饰句。
   */

  onMount(() => {
    void intel.wire();
  });

  /** 主题图标（注册表驱动卡片，图标按 id 取）。 */
  const TOPIC_ICONS: Record<string, string> = {
    models: 'M12 3l9 5-9 5-9-5zM3 12l9 5 9-5M3 16l9 5 9-5',
    ai: 'M12 3l1.8 4.6L18 9.4l-4.2 1.8L12 16l-1.8-4.8L6 9.4l4.2-1.8zM19 15l.9 2.3L22 18l-2.1.9L19 21l-.9-2.1L16 18l2.1-.7z',
    'model-news': 'M9 17H7a5 5 0 110-10h2m6 10h2a5 5 0 100-10h-2M8 12h8',
    tech: 'M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M7 14h10',
  };

  function topicIcon(id: string): string {
    return TOPIC_ICONS[id] ?? 'M12 3l1.8 4.6L18 9.4l-4.2 1.8L12 16l-1.8-4.8L6 9.4l4.2-1.8z';
  }
</script>

<section class="flex h-full flex-col">
  {#if intel.active === null}
    <!-- 入口：主题卡片网格 -->
    <header class="flex shrink-0 items-center justify-between px-6 pb-4 pt-5">
      <h1 class="qx-page-title">情报</h1>
      <div class="flex items-center gap-3">
        <span class="text-xs text-muted" data-testid="intel-updated">
          {intel.lastFetchAt ? `${formatTime(intel.lastFetchAt)} 更新` : ''}
        </span>
        <button
          class="qx-btn qx-btn-sm qx-btn-outline"
          onclick={() => void intel.openStatus()}
          data-testid="intel-status-open"
        >
          状态
        </button>
      </div>
    </header>
    <!-- 今日要览（docs/17 §7.5.2-1）：第一屏 = 今天的杂志，统计卡退居其下 -->
    <FrontPage />
    <div
      class="grid min-h-0 flex-1 content-start gap-4 overflow-y-auto px-6 pb-6"
      style="grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));"
      aria-live="polite"
    >
      {#each intel.topics as t (t.id)}
        <button
          class="qx-card p-5 text-left transition-colors hover:border-accent/40"
          onclick={() => void intel.open(t.id)}
          data-testid={'intel-card-' + t.id}
        >
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium">{t.label}</span>
            <svg
              viewBox="0 0 24 24"
              class="size-4 text-accent"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d={topicIcon(t.id)} />
            </svg>
          </div>
          <div class="mt-3 font-mono text-2xl font-semibold tracking-tight">
            {t.todayCount}
            {#if t.id === 'models'}
              <span class="text-xs font-normal text-muted">家族</span>
            {/if}
          </div>
          <div class="mt-1.5 flex items-center gap-1.5 text-xs text-muted">
            {#if t.fetching || intel.refreshing}
              <span class="size-1.5 rounded-full bg-accent" class:animate-pulse={t.fetching}></span>
              抓取中…
            {:else if t.lastFetchAt === null}
              <span class="size-1.5 rounded-full bg-muted/50"></span>
              未抓
            {:else}
              <span class="size-1.5 rounded-full" class:bg-ok={t.ok} class:bg-warning={!t.ok}
              ></span>
              {formatTime(t.lastFetchAt)}{t.ok ? '' : ' ⚠'}
            {/if}
          </div>
        </button>
      {/each}
      <button
        class="qx-card p-5 text-left transition-colors hover:border-accent/40"
        onclick={() => void intel.open('favorites')}
        data-testid="intel-card-favorites"
      >
        <div class="flex items-center justify-between">
          <span class="text-sm font-medium">收藏</span>
          <svg
            viewBox="0 0 24 24"
            class="size-4 text-accent"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M6 3h12v18l-6-4-6 4z" />
          </svg>
        </div>
        <div class="mt-3 font-mono text-2xl font-semibold tracking-tight">
          {intel.favorites.length}
        </div>
        <div class="mt-1.5 text-xs text-muted">条内容</div>
      </button>
    </div>
  {:else if intel.active === 'favorites'}
    <FavoritesView />
  {:else}
    <FeedView />
  {/if}
</section>

<!-- 情报状态页（点「状态」按钮打开） -->
<StatusDialog />
