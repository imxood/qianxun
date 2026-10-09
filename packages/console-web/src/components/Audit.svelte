<script lang="ts">
  import { api, type AuditEvent } from '../lib/api';
  import { describeEvent, eventTone, isFailure } from '../lib/audit';
  import { stamp } from '../lib/fmt';
  import { toast } from '../lib/toast.svelte';

  /** 审计是唯一的事件出口（docs/20 §2.1 规则 5），所以只有它需要筛选与导出。 */
  const POLL_MS = 3000;
  const PAGE = 60;

  // 筛选按前缀走，与服务端 audit_query 的 LIKE 前缀一致；新增事件只要落在这四个
  // 前缀下就自动进筛选，不用改这里。
  const FILTERS = [
    { value: '', label: '全部' },
    { value: 'login', label: '登录' },
    { value: 'pair', label: '配对' },
    { value: 'instance', label: '实例' },
    { value: 'device', label: '设备' },
  ];

  let events = $state<AuditEvent[]>([]);
  let filter = $state('');
  let loaded = $state(false);
  let failure = $state('');
  let lastSync = $state(0);
  let cursor = $state<string | null>(null);
  /** 连续失败的计数按 (事件, 主体) 归并，行尾标「连续 N/5」（docs/20 §4.5）。
   *  用普通对象而不是 Map：每次换页整体替换，从不就地改，不需要响应式 Map。 */
  let fails = $state<Record<string, number>>({});

  function streakOf(event: AuditEvent): number | null {
    if (!isFailure(event.event)) return null;
    const key = `${event.event}|${event.deviceId ?? event.instanceId ?? ''}`;
    const current = fails[key];
    if (current == null) return null;
    return current >= 3 ? current : null;
  }

  async function refresh(type = filter, append = false): Promise<void> {
    try {
      const page = await api.audit({
        type: type || undefined,
        cursor: append && cursor ? cursor : undefined,
      });
      events = append ? [...events, ...page.events] : page.events;
      cursor = page.nextCursor;
      lastSync = Date.now();
      failure = '';
      if (!append) {
        const next: Record<string, number> = {};
        for (const event of events) {
          if (!event.event.endsWith('.fail')) continue;
          const key = `${event.event}|${event.deviceId ?? event.instanceId ?? ''}`;
          next[key] = (next[key] ?? 0) + 1;
        }
        fails = next;
      }
    } catch (cause) {
      failure = cause instanceof Error ? cause.message : '审计读取失败';
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    // filter 必须在这里同步读一次才成为依赖：$effect 只接受一个参数，
    // 换查询集靠的是「依赖变了就重跑」，不是给 effect 传参。
    const current = filter;
    void refresh(current);
    const timer = setInterval(() => void refresh(current), POLL_MS);
    return () => clearInterval(timer);
  });

  const liveSeconds = $derived(lastSync ? Math.floor((Date.now() - lastSync) / 1000) : 0);

  function exportCsv(): void {
    window.location.href = api.auditCsvUrl({ type: filter || undefined });
    toast('已导出 CSV');
  }
</script>

<div class="pagehead">
  <div class="flex items-center gap-3">
    <h1>审计</h1>
    <span class="live-pill">
      <span class="dot {failure ? 'dot-warn' : 'dot-on'}"></span>
      {#if failure}连接中断{:else}live · {liveSeconds}s{/if}
    </span>
  </div>
  <div class="flex gap-2">
    <select
      class="btn btn-ghost btn-sm appearance-none focus-ring"
      value={filter}
      onchange={(e) => (filter = e.currentTarget.value)}
    >
      {#each FILTERS as option (option.value)}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
    <button class="btn btn-ghost btn-sm focus-ring" onclick={exportCsv}>导出</button>
  </div>
</div>

{#if failure}
  <div
    class="mb-4 rounded-lg border border-warn/45 bg-warn/10 px-3 py-2.5 text-[13px] text-warn"
    role="alert"
  >
    连接中断，正在重试——下方数据可能已过期。
  </div>
{/if}

{#if !loaded}
  <div class="card p-8 text-center text-sm text-muted">加载中…</div>
{:else if events.length === 0}
  <div class="card px-6 py-14 text-center text-[13px] text-muted">没有符合条件的事件</div>
{:else}
  <div class="card overflow-hidden">
    {#each events as event (event.id)}
      <div class="tl-item">
        <span class="tl-time">{stamp(event.at)}</span>
        <span class="tl-badge {eventTone(event.event)}" title={event.event}>{event.event}</span>
        <span class="min-w-0 flex-1">
          {describeEvent(event)}
          {#if streakOf(event)}（连续 {streakOf(event)}/5）{/if}
        </span>
      </div>
    {/each}
  </div>
  {#if cursor}
    <div class="mt-3 flex justify-center">
      <button class="btn btn-ghost btn-sm focus-ring" onclick={() => void refresh(filter, true)}
        >加载更早的 {PAGE} 条</button
      >
    </div>
  {/if}
{/if}
