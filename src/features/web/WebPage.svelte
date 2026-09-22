<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { nav } from '../../stores/nav.svelte';
  import { settings } from '../../stores/settings.svelte';
  import { web } from '../../stores/web.svelte';

  /**
   * 联网搜索页（R001）。结果以结构化列表渲染（标题/链接/摘要），
   * Markdown 汇总放可复制代码块——不把未信任内容注入 innerHTML。
   */

  const engines = $derived(settings.current?.web.engines ?? []);

  // 引擎下拉初值 = settings.web.defaultEngine（settings 加载后对齐一次）。
  $effect(() => {
    if (web.engineId === '' && engines.length > 0) {
      web.engineId = settings.current?.web.defaultEngine ?? engines[0]?.id ?? '';
    }
  });

  function submit(): void {
    void web.search();
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter') submit();
  }

  function open(url: string): void {
    void openUrl(url).catch(() => {});
  }
</script>

<section class="flex h-full flex-col">
  <header class="flex shrink-0 items-center gap-4 px-6 pb-3 pt-5">
    <h1 class="qx-page-title">联网</h1>
  </header>

  <!-- 搜索工具行 -->
  <div class="flex shrink-0 items-center gap-2 px-6">
    <input
      class="h-9 min-w-0 flex-1 rounded-lg border border-line bg-surface px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
      placeholder="搜索网页内容（回车或点按钮）"
      bind:value={web.query}
      onkeydown={onKeydown}
      data-testid="web-query"
    />
    <select
      class="h-9 rounded-lg border border-line bg-surface px-2 text-sm"
      bind:value={web.engineId}
      data-testid="web-engine"
      aria-label="搜索引擎"
    >
      {#each engines as engine (engine.id)}
        <option value={engine.id}>{engine.id}</option>
      {/each}
    </select>
    <button
      class="h-9 shrink-0 rounded-lg bg-accent px-4 text-sm text-white disabled:opacity-50"
      onclick={submit}
      disabled={web.busy}
      data-testid="web-search"
    >
      {web.busy ? '搜索中…' : '搜索'}
    </button>
  </div>

  <!-- DSH 未运行失败态：给出去向引导，不白屏 -->
  {#if !web.available}
    <div class="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 text-muted">
      <p data-testid="web-unavailable">DSH 未运行，联网搜索不可用。</p>
      <button
        class="rounded-lg border border-line px-4 py-2 text-sm hover:bg-accent-soft/50"
        onclick={() => nav.go('env')}
        data-testid="web-go-env"
      >
        去环境页启动 DSH
      </button>
    </div>
  {:else}
    <!-- 结果区：aria-live 播报状态（ADR-016） -->
    <div class="min-h-0 flex-1 overflow-y-auto px-6 pb-6 pt-4" aria-live="polite">
      {#if web.error}
        <p
          class="rounded-lg border border-danger/40 bg-danger/5 px-3 py-2 text-sm text-danger"
          data-testid="web-error"
        >
          {web.error}
        </p>
      {:else if web.busy}
        <p class="text-sm text-muted" data-testid="web-busy">搜索中…</p>
      {:else if web.items.length === 0}
        <p class="text-sm text-muted" data-testid="web-empty">
          {web.done
            ? '没有搜索到结果：换个关键词或引擎再试（百度反爬较严，建议 duckduckgo / bing）。'
            : '输入关键词开始搜索。'}
        </p>
      {:else}
        <ol class="flex flex-col gap-3">
          {#each web.items as item, index (item.url)}
            <li
              class="rounded-lg border border-line bg-surface p-3"
              data-testid={'web-item-' + String(index)}
            >
              <button
                class="text-left text-sm font-medium text-accent hover:underline"
                onclick={() => open(item.url)}
              >
                {item.title}
              </button>
              <p class="mt-0.5 truncate text-xs text-muted">{item.url}</p>
              {#if item.snippet}
                <p class="mt-1 text-sm">{item.snippet}</p>
              {/if}
            </li>
          {/each}
        </ol>
        {#if web.markdown}
          <details class="mt-4">
            <summary class="cursor-pointer text-sm text-muted" data-testid="web-markdown-toggle">
              Markdown 汇总（可复制）
            </summary>
            <pre
              class="mt-2 overflow-x-auto rounded-lg border border-line bg-surface p-3 text-xs"
              data-testid="web-markdown">{web.markdown}</pre>
          </details>
        {/if}
      {/if}
    </div>
  {/if}
</section>
