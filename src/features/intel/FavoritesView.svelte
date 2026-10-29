<script lang="ts">
  import { call } from '../../lib/ipc';
  import type { Article } from '../../lib/ipc/contract';
  import { intel } from '../../stores/intel.svelte';
  import { formatTime } from './format';

  /**
   * 收藏视图：按收藏日期分组；文章/feed 条目统一行形态。
   * 收藏是快照拷贝（docs/17 §8.3），不随历史清理变化。
   * 旧榜单时代的 model 收藏快照已失效（M3），渲染时过滤。
   */

  /** 有效收藏（article / feed 快照；榜单时代残留的空条目跳过）。 */
  const validFavorites = $derived(intel.favorites.filter((fav) => fav.article || fav.feed));

  const groups = $derived.by(() => {
    const out: Array<[string, typeof validFavorites]> = [];
    for (const fav of validFavorites) {
      const date = new Date(fav.savedAt);
      const pad = (n: number) => String(n).padStart(2, '0');
      const key = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
      const last = out[out.length - 1];
      // favorites 按收藏时间倒序（新收藏插队首），同日天然相邻。
      if (last && last[0] === key) last[1].push(fav);
      else out.push([key, [fav]]);
    }
    return out;
  });

  async function open(url: string): Promise<void> {
    if (!url) return;
    try {
      await call('window_open_external', { url, newWindow: false });
    } catch {
      /* 打开失败不打断 */
    }
  }

  /** 收藏的文章：切到对应主题的文章流并打开阅读视图。 */
  async function openArticleFav(article: Article): Promise<void> {
    await intel.open(article.topicId);
    intel.feedTab = 'digest';
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
  <h1 class="qx-page-title">收藏</h1>
  <span class="ml-2 font-mono text-xs text-muted">{validFavorites.length}</span>
</header>

<div class="min-h-0 flex-1 overflow-y-auto px-6 pb-6" aria-live="polite">
  {#if validFavorites.length === 0}
    <p class="pt-10 text-center text-sm text-muted" data-testid="intel-favorites-empty">空</p>
  {:else}
    {#each groups as [date, favorites] (date)}
      <h2 class="qx-h mt-4 first:mt-0">{date}</h2>
      <div class="qx-card mt-2 divide-y divide-line/60">
        {#each favorites as fav (fav.id)}
          {@const topicLabel = intel.topic(fav.topicId)?.label ?? fav.topicId}
          <article class="flex items-start gap-3 px-4 py-3">
            <div class="min-w-0 flex-1">
              {#if fav.article}
                {@const article = fav.article}
                <button
                  class="block w-full truncate text-left text-sm font-medium transition-colors hover:text-accent"
                  title={article.title}
                  onclick={() => void openArticleFav(article)}
                  data-testid={'intel-fav-article-' + article.id}
                >
                  {article.title}
                </button>
                <div class="mt-1 flex items-center gap-2 text-xs text-muted">
                  <span class="qx-badge bg-accent-soft/60 text-accent">{topicLabel}</span>
                  <span class="truncate">{article.lead}</span>
                  <span class="shrink-0">{formatTime(fav.savedAt)} 收</span>
                </div>
              {:else if fav.feed}
                <button
                  class="block w-full truncate text-left text-sm font-medium transition-colors hover:text-accent"
                  title={fav.feed.title}
                  onclick={() => void open(fav.feed?.url ?? '')}
                >
                  {fav.feed.title}
                </button>
                <div class="mt-1 flex items-center gap-2 text-xs text-muted">
                  <span class="qx-badge bg-accent-soft/60 text-accent">{topicLabel}</span>
                  <span>{formatTime(fav.savedAt)} 收</span>
                </div>
              {/if}
            </div>
            <button
              class="qx-icon-btn mt-0.5"
              aria-label="取消收藏"
              onclick={() => void intel.unsave(fav.id)}
              data-testid={'intel-fav-remove-' + fav.id}
            >
              <svg
                viewBox="0 0 24 24"
                class="size-4 text-accent"
                fill="currentColor"
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M12 17.3l-6.2 3.6 1.7-7L2 9.2l7.1-.6L12 2l2.9 6.6 7.1.6-5.5 4.7 1.7 7z" />
              </svg>
            </button>
          </article>
        {/each}
      </div>
    {/each}
  {/if}
</div>
