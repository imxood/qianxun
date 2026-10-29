<script lang="ts">
  import { intel, DIGEST_STAGES, stageLabel, type StatusRow } from '../../stores/intel.svelte';
  import { sourceLabel } from '../../stores/intel.svelte';
  import { formatTime } from './format';

  /**
   * 情报状态页（点「状态」按钮打开）：LLM 通道 + 各主题抓取/出刊全状态。
   * 抓取中显示正在请求的 URL（intel://fetching 实时推送）。文案简洁。
   */

  /** 抓取状态摘要（一行）。 */
  function fetchLine(row: StatusRow): string {
    const t = row.topic;
    if (t.fetching || (t.active?.length ?? 0) > 0) return '抓取中…';
    if (t.lastFetchAt === null) return '未抓';
    const n = `${t.todayCount} 条`;
    return `${formatTime(t.lastFetchAt)} · ${n}${t.ok ? '' : ' · ⚠ 部分源失败'}`;
  }

  /** 出刊状态摘要（终态/未开始，一行）。 */
  function digestSummary(row: StatusRow): string {
    const d = row.digest;
    if (d && intel.digestRunningOf(d)) return d.message;
    if (row.articles && row.articles.articles.length > 0) {
      return `${row.articles.articles.length} 篇 · ${row.articles.model}`;
    }
    if (d?.stage === 'llm-unavailable') return d.message;
    if (d?.stage === 'failed') return d.message;
    return '未出刊';
  }

  /** 步骤状态（时间线）。 */
  function stepState(row: StatusRow, index: number): string {
    const d = row.digest;
    if (!d) return 'pending';
    if (d.stage === 'failed') return 'failed';
    if (d.stage === 'done') return 'done';
    if (d.stage === 'llm-unavailable') return index === 0 ? 'done' : 'pending';
    const current = DIGEST_STAGES.indexOf(d.stage as (typeof DIGEST_STAGES)[number]);
    if (current < 0) return 'pending';
    if (index < current) return 'done';
    if (index === current) return 'active';
    return 'pending';
  }

  const dshReady = $derived(intel.dshPhase === 'ready');
</script>

{#if intel.statusOpen}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-6">
    <button
      class="absolute inset-0 cursor-default"
      aria-label="关闭状态页"
      onclick={() => intel.closeStatus()}
    ></button>
    <div
      class="relative max-h-[82vh] w-full max-w-xl overflow-y-auto rounded-xl border border-line bg-card p-5 shadow-2xl"
      role="dialog"
      aria-label="情报状态"
      data-testid="intel-status-dialog"
    >
      <header class="flex items-center justify-between">
        <h1 class="text-base font-semibold">情报状态</h1>
        <button class="qx-icon-btn" aria-label="关闭" onclick={() => intel.closeStatus()}>
          <svg
            viewBox="0 0 24 24"
            class="size-4"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            aria-hidden="true"
          >
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      </header>

      <!-- LLM 通道 -->
      <section class="mt-4">
        <h2 class="text-xs font-medium text-muted">LLM 通道</h2>
        {#if dshReady}
          <p class="mt-1 text-sm" data-testid="intel-status-dsh">
            DSH 托管：运行中
            {#if intel.statusRows.some((r) => r.articles?.model)}
              · {intel.statusRows.find((r) => r.articles?.model)?.articles?.model}
            {/if}
          </p>
        {:else}
          <p class="mt-1 text-sm text-muted" data-testid="intel-status-dsh">
            DSH 托管：未随千寻运行 · 出刊走 qx-llm 配置直连
          </p>
        {/if}
      </section>

      <!-- 各主题 -->
      {#each intel.statusRows as row (row.topic.id)}
        {@const running = row.digest ? intel.digestRunningOf(row.digest) : false}
        <section class="mt-5 border-t border-line pt-4">
          <div class="flex items-baseline justify-between gap-3">
            <h2 class="text-sm font-semibold">{row.topic.label}</h2>
            <span
              class="font-mono text-xs text-muted"
              data-testid={'intel-status-fetch-' + row.topic.id}
            >
              {fetchLine(row)}
            </span>
          </div>

          <!-- 正在请求的 URL（并发中的每源一行，intel://fetching 推送） -->
          {#if (row.topic.active?.length ?? 0) > 0}
            <ul class="mt-1.5 space-y-0.5" data-testid={'intel-status-requests-' + row.topic.id}>
              {#each row.topic.active as activity (activity.sourceId)}
                <li class="flex items-baseline gap-2 text-xs">
                  <span class="shrink-0 animate-pulse text-accent">●</span>
                  <span class="shrink-0 text-muted">正在请求 {sourceLabel(activity.sourceId)}</span>
                  <span class="truncate font-mono text-muted" title={activity.url}>
                    {activity.url}
                  </span>
                </li>
              {/each}
            </ul>
          {/if}

          {#if running}
            <!-- 进行中：展开流水线 -->
            <ol class="mt-2 space-y-1" aria-live="polite">
              {#each DIGEST_STAGES as stage, i (stage)}
                {@const state = stepState(row, i)}
                <li class="flex items-center gap-2.5 text-sm">
                  <span
                    class="flex size-5 shrink-0 items-center justify-center rounded-full text-[10px]
                             {state === 'done'
                      ? 'bg-ok/15 text-ok'
                      : state === 'active'
                        ? 'bg-accent-soft text-accent'
                        : state === 'failed'
                          ? 'bg-danger/15 text-danger'
                          : 'bg-surface text-muted'}"
                  >
                    {#if state === 'done'}
                      ✓
                    {:else if state === 'active'}
                      <span class="size-2 animate-pulse rounded-full bg-accent"></span>
                    {:else}
                      {i + 1}
                    {/if}
                  </span>
                  <span class={state === 'pending' ? 'text-muted' : ''}>{stageLabel(stage)}</span>
                  {#if state === 'active' && row.digest && row.digest.total > 1}
                    <span class="ml-auto font-mono text-xs text-muted">
                      {row.digest.done}/{row.digest.total}
                    </span>
                  {/if}
                </li>
              {/each}
            </ol>
            {#if row.digest}
              <p class="mt-1.5 truncate text-xs text-muted">{row.digest.message}</p>
            {/if}
          {:else}
            <p
              class="mt-1 text-sm {row.digest?.stage === 'llm-unavailable' ||
              row.digest?.stage === 'failed'
                ? 'text-danger'
                : ''}"
              data-testid={'intel-status-digest-' + row.topic.id}
            >
              出刊 {digestSummary(row)}
            </p>
            {#if row.articles && row.articles.articles.length > 0}
              <p class="mt-0.5 truncate text-xs text-muted">
                {row.articles.articles
                  .slice()
                  .sort((a, b) => b.importance - a.importance)
                  .slice(0, 3)
                  .map((a) => a.title)
                  .join('；')}
              </p>
            {/if}
            <button
              class="qx-btn qx-btn-sm qx-btn-outline mt-2"
              onclick={() => void intel.startDigest(row.topic.id)}
              data-testid={'intel-status-redigest-' + row.topic.id}
            >
              重新出刊
            </button>
          {/if}
        </section>
      {/each}
    </div>
  </div>
{/if}
