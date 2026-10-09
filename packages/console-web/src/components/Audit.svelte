<script lang="ts">
  import { api, type AuditEvent } from '../lib/api';
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
    if (!event.event.endsWith('.fail')) return null;
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

  function tone(event: AuditEvent): string {
    if (event.event.endsWith('.fail')) return 'tl-fail';
    if (event.event.startsWith('login.ok') || event.event.endsWith('.ok')) return 'tl-ok';
    return 'tl-info';
  }

  /** 一句话说清「谁、做了什么、在哪」。
   *
   *  这张表按**服务端真实发出的事件名**写，不是按设想中的名字——曾经这里写着
   *  `pair.ok` / `instance.up`，而服务端根本不发这两个，落到 default 就把
   *  `pair.submit_ok` 之类的原始标识符直接印给了用户（docs/20 §4.5）。
   *  detail 里没有的字段就不编，主体缺了就退到 id，再缺就写「设备」。 */
  function describe(event: AuditEvent): string {
    const detail = event.detail;
    const who = (detail['name'] as string) ?? null;
    const ip = (detail['ip'] as string) ?? null;
    const tail = ip ? ` · ${ip}` : '';
    const device = event.deviceId ?? '设备';
    const instance = event.instanceId ?? '实例';
    const via = (detail['via'] as string) ?? '';
    // 同一个动作不写两条事件（db.rs 一次动作只写一条），来源记在 detail.via。
    const where = via === 'console' ? '控制台' : via === 'instance' ? '实例' : '命令行';
    switch (event.event) {
      case 'login.ok':
        return `${who ?? '用户'} 登录成功${tail}`;
      case 'login.fail':
        return `${who ?? '用户'} 密码错误${tail}`;
      case 'login.locked':
        return `${who ?? '用户'} 触发限流锁定${tail}`;
      // 配对：发码、手机兑换、手机进场三段。
      case 'pair.create':
        return `${where}生成配对码`;
      case 'pair.submit_ok':
        return `${device} 兑换配对码成功${tail}`;
      case 'pair.submit_fail':
        return `配对码错误${tail}`;
      case 'pair.locked':
        return `配对提交触发限流锁定${tail}`;
      case 'pair.grant':
        return `${device} 已接入`;
      // 实例
      case 'instance.add':
        return `${instance} 已登记`;
      case 'instance.rotate':
        return `${instance} 令牌已轮换`;
      case 'instance.offline':
        return `${instance} 掉线`;
      // 设备
      case 'device.enter':
        return `${device} 建立连接${tail}`;
      case 'device.enter_fail':
        return `设备凭据无效${tail}`;
      case 'device.revoke':
        return `${device} 已被${where}吊销`;
      case 'device.restore':
        return `${device} 已恢复`;
      case 'device.revoke_all':
        return '吊销全部设备';
      case 'device.rotate':
        return `${device} 凭据已轮换`;
      case 'device.import':
        return `${device} 由命令行导入`;
      default:
        // 不认识的事件不编句子，但也不能装作不存在：原始键仍然可见、可搜索。
        return `${event.event}（未分类事件）`;
    }
  }

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
        <span class="tl-badge {tone(event)}" title={event.event}>{event.event}</span>
        <span class="min-w-0 flex-1">
          {describe(event)}
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
