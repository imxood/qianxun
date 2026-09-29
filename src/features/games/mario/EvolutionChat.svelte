<script module lang="ts">
  /**
   * 进化对话台列(docs/16 §4):常驻主列,非浮窗。
   * 视觉规范 §1.4:Swiss 极简,卡片非气泡,四色语义,mono 数字,动效仅两处。
   */
</script>

<script lang="ts">
  import { chatGroups, type ChatMsg, type ChatPatch } from './chat';

  interface Props {
    messages: readonly ChatMsg[];
    status: {
      mode: string;
      layaOk: boolean;
      qwenOk: boolean;
      queueDepth: number;
      analyzing: boolean;
      breaker: boolean;
      evolve: 'auto' | 'step' | 'pause';
      won: number;
      runs: number;
      maxX: number;
    };
    onstepadvance?: () => void;
    onevolve?: (m: 'auto' | 'step' | 'pause') => void;
  }

  let { messages, status, onstepadvance, onevolve }: Props = $props();

  let scrollEl: HTMLElement | null = null;
  let atBottom = $state(true);
  let unseen = $state(0);
  let folded = $state(false);

  const groups = $derived.by(() => chatGroups([...messages]).slice(-30));

  // 锚定(docs/16 §4.4):只依赖 length + atBottom;锁定时计未读
  $effect(() => {
    void messages.length;
    if (atBottom && scrollEl !== null) {
      scrollEl.scrollTop = scrollEl.scrollHeight;
      unseen = 0;
    } else if (!atBottom) {
      unseen += 1;
    }
  });

  function onScroll(): void {
    const el = scrollEl;
    if (el === null) return;
    atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    if (atBottom) unseen = 0;
  }

  function scrollToBottom(): void {
    if (scrollEl === null) return;
    atBottom = true; // 先同步,防 scroll 事件回环
    scrollEl.scrollTop = scrollEl.scrollHeight;
    unseen = 0;
  }

  const sepText = (m: ChatMsg): string => {
    const r = m.result ?? 'live';
    if (r === 'live') return `#${m.iter ?? '?'} · ${m.mode ?? ''} · 进行中`;
    if (r === 'win')
      return `#${m.iter ?? '?'} · ${m.mode ?? ''} · 通关 · ${m.score?.toFixed(0) ?? ''}分`;
    return `#${m.iter ?? '?'} · ${m.mode ?? ''} · ${m.deaths ?? 0}死 · ${m.maxXCol?.toFixed(0) ?? ''}列`;
  };

  const fmtPatch = (patch: ChatPatch): string =>
    Object.entries(patch)
      .map(([k, v]) => `${k} ${String(v.from)}→${String(v.to)}`)
      .join(' · ');
</script>

<div
  class="relative flex h-full min-h-0 w-[380px] flex-none flex-col border-l border-line bg-[#0d0f14]"
  data-testid="evolution-chat"
>
  <!-- 列头常驻条(≤7 字段) -->
  <div class="flex h-9 flex-none items-center gap-2 border-b border-line px-3">
    <span class="text-[11px] font-semibold tracking-wider text-slate-300">进化</span>
    <select
      class="rounded border border-line bg-[#11141c] px-1 py-0.5 text-[11px] text-slate-300"
      value={status.evolve}
      onchange={(e) =>
        onevolve?.((e.currentTarget as HTMLSelectElement).value as Props['status']['evolve'])}
      data-testid="evo-mode-select"
      aria-label="进化三态"
    >
      <option value="auto">自动</option>
      <option value="step">单步</option>
      <option value="pause">暂停</option>
    </select>
    <span class="ml-auto flex items-center gap-1.5" title="Laya / Qwen 探活">
      <span class="h-1.5 w-1.5 rounded-full {status.layaOk ? 'bg-emerald-400' : 'bg-slate-600'}"
      ></span>
      <span
        class="h-1.5 w-1.5 rounded-full {status.qwenOk
          ? 'bg-emerald-400'
          : status.breaker
            ? 'bg-red-400'
            : 'bg-slate-600'}"
        title={status.breaker
          ? 'Qwen 熔断,探活恢复自启'
          : status.qwenOk
            ? 'Qwen 在线'
            : 'Qwen 离线'}
      ></span>
      {#if status.analyzing || status.queueDepth > 0}
        <span class="text-[11px] text-sky-400" data-testid="chat-queue">析 {status.queueDepth}</span
        >
      {/if}
      <span class="text-[11px] text-muted mono"
        >{status.runs > 0 ? `${status.won}/${status.runs}` : ''}</span
      >
    </span>
  </div>

  {#if folded}
    <button
      class="flex-1 cursor-pointer text-center text-[11px] text-muted"
      onclick={() => {
        folded = false;
        setTimeout(scrollToBottom, 0);
      }}
    >
      进化台(点击展开)
    </button>
  {:else}
    <!-- 消息流 -->
    <div
      class="min-h-0 flex-1 overflow-y-auto px-3 py-2"
      bind:this={scrollEl}
      onscroll={onScroll}
      data-testid="chat-stream"
    >
      {#if messages.length === 0}
        <div class="mt-16 text-center text-[12px] text-muted">跑一局,Qwen 的复盘会出现在这里</div>
      {/if}
      {#each groups as g (g.runId)}
        {@const sep = g.msgs.find((m) => m.kind === 'sep')}
        {@const ended = sep?.result !== 'live'}
        <div class="mb-4 {ended ? 'opacity-70' : ''}">
          <!-- 局分隔符 -->
          <div class="mb-2 mt-1 flex items-center gap-2">
            <div class="h-px flex-1 bg-line"></div>
            <span class="text-[10px] tracking-wider text-muted uppercase mono">
              {sep === undefined ? `${g.runId} · 进行中` : sepText(sep)}
            </span>
            <div class="h-px flex-1 bg-line"></div>
          </div>
          {#each g.msgs as m (m.id)}
            {#if m.kind === 'death'}
              <div class="py-0.5 text-[12px] text-slate-300">
                <span class="text-slate-500">第{m.attempt}死</span>
                {m.cause === 'pit' ? '掉坑' : m.cause}@{m.col}
                <span class="mono text-slate-500">· {m.pct}%</span>
              </div>
            {:else if m.kind === 'analysis'}
              <div
                class="my-1.5 rounded-lg border border-line bg-surface/60 px-3 py-2
                  {m.phase === 'ok' ? 'border-l-2 border-l-violet-400' : ''}
                  {m.phase === 'fail' || m.phase === 'queued'
                  ? 'border-l-2 border-l-amber-400/70'
                  : ''}"
                data-testid="chat-analysis-{m.key}"
              >
                <div
                  class="mb-0.5 flex items-center gap-2 text-[10px] tracking-wider text-slate-500 uppercase"
                >
                  {m.phase === 'ok' ? 'Qwen' : '分析'}
                  {#if m.phase === 'ok' && m.latencyMs}
                    <span class="mono">{(m.latencyMs / 1000).toFixed(1)}s</span>
                  {/if}
                  <span class="truncate">{m.reason}</span>
                </div>
                {#if m.phase === 'ok'}
                  <div class="text-[12px] leading-relaxed text-slate-200">{m.text}</div>
                  {#if m.meta && m.meta.length > 0}
                    <div class="mt-1 text-[11px] text-muted mono">{m.meta.join(' · ')}</div>
                  {/if}
                {:else if m.phase === 'fail'}
                  <div class="text-[12px] text-amber-400">分析失败,已弃析</div>
                {:else}
                  <div class="flex items-center gap-2 text-[12px] text-slate-400">
                    <span class="inline-block h-1 w-16 overflow-hidden rounded bg-[#1a1e29]">
                      <span class="block h-full w-1/3 animate-pulse rounded bg-sky-400/70"></span>
                    </span>
                    {m.phase === 'running' ? '分析中…' : '排队中'}
                  </div>
                {/if}
              </div>
            {:else if m.kind === 'proposal'}
              <div
                class="my-1.5 rounded-lg border border-line bg-surface/60 px-3 py-2"
                data-testid="chat-proposal"
              >
                <div
                  class="mb-0.5 flex items-center gap-2 text-[10px] tracking-wider text-slate-500 uppercase"
                >
                  提案 · {m.patchKind === 'design' ? '消息设计' : '数值'}
                  {#if m.verdict === 'commit'}<span class="text-emerald-400">✓ 转正</span>{/if}
                  {#if m.verdict === 'rollback'}<span class="text-amber-400">✗ 回滚</span>{/if}
                </div>
                <div class="text-[12px] text-slate-200 mono">{fmtPatch(m.patch ?? {})}</div>
                {#if m.gated && m.verdict == null}
                  <button
                    class="mt-1.5 cursor-pointer rounded-md bg-sky-600 px-2 py-0.5 text-[11px] text-white hover:bg-sky-500"
                    onclick={() => onstepadvance?.()}
                    data-testid="btn-step-advance"
                  >
                    装填下局 ▸
                  </button>
                {/if}
              </div>
            {:else if m.kind === 'verdict'}
              <div
                class="py-0.5 text-[12px] mono {m.ok ? 'text-emerald-400' : 'text-amber-400'}"
                data-testid="chat-verdict"
              >
                {m.text}
              </div>
            {/if}
          {/each}
        </div>
      {/each}
    </div>
    <!-- 未读浮标 -->
    {#if !atBottom && unseen > 0}
      <button
        class="absolute bottom-12 right-4 cursor-pointer rounded-full bg-sky-600 px-3 py-1 text-[11px] text-white shadow"
        onclick={scrollToBottom}
      >
        ↓ {unseen} 条新消息
      </button>
    {/if}
    <button
      class="h-5 flex-none cursor-pointer border-t border-line text-[10px] text-muted hover:text-slate-300"
      onclick={() => (folded = true)}
    >
      收起
    </button>
  {/if}
</div>

<style>
  .mono {
    font-family: var(--mono, ui-monospace, monospace);
  }
</style>
