<script lang="ts">
  import { intel } from '../../stores/intel.svelte';

  /**
   * 今日要览（docs/17 §7.5.2-1）：情报入口第一屏。
   * 头条大卡 = 全 Feed 主题 importance 最高一篇；要目 = 其余 top5 一行一条。
   * 数据由 store.loadFrontPage 聚合（各主题当日/最近一期）。
   */
  const all = $derived(
    intel.frontRows.flatMap((row) =>
      (row.day?.articles ?? []).map((article) => ({ article, row })),
    ),
  );
  // 后端落盘已按 importance 降序。
  const head = $derived(all[0] ?? null);
  const rest = $derived(all.slice(1, 6));

  function open(entry: {
    article: (typeof all)[number]['article'];
    row: (typeof all)[number]['row'];
  }): void {
    void intel.openArticle(entry.article, entry.row.topicId);
  }
</script>

{#if all.length > 0}
  <section
    class="mx-6 mb-4 rounded-xl border border-line bg-card p-4"
    data-testid="intel-frontpage"
  >
    <div class="flex items-baseline justify-between">
      <h2 class="text-sm font-semibold">今日要览</h2>
      <span class="text-xs text-muted">共 {all.length} 篇</span>
    </div>

    {#if head}
      <button
        class="mt-2 block w-full rounded-lg px-2 py-2 text-left transition-colors hover:bg-surface"
        onclick={() => open(head)}
        data-testid="intel-frontpage-headline"
      >
        <span class="qx-badge text-accent">头条 · {head.row.label}</span>
        <h3 class="mt-1.5 text-lg font-semibold leading-snug">{head.article.title}</h3>
        {#if head.article.lead}
          <p class="mt-1 line-clamp-2 text-sm text-muted">{head.article.lead}</p>
        {/if}
      </button>
    {/if}

    {#if rest.length > 0}
      <ul class="mt-1 divide-y divide-line border-t border-line">
        {#each rest as entry (entry.article.id)}
          <li>
            <button
              class="flex w-full items-center gap-2.5 py-2 text-left transition-colors hover:bg-surface"
              onclick={() => open(entry)}
              data-testid="intel-frontpage-item-{entry.article.id}"
            >
              <span class="qx-badge shrink-0 text-muted">{entry.row.label}</span>
              <span class="min-w-0 flex-1 truncate text-sm">{entry.article.title}</span>
              <span
                class="shrink-0 font-mono text-xs text-muted"
                title="重要性 {entry.article.importance}"
              >
                {'★'.repeat(entry.article.importance)}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}
