<script lang="ts">
  import { api, type Overview } from '../lib/api';
  import { bytes, percent, relativeTime } from '../lib/fmt';
  import { toast } from '../lib/toast.svelte';

  /** 概览一次请求拿全部：KPI + 水位 + 实例 + 流量，分四个请求会让 3 秒轮询
   *  的 round-trip 数翻倍，且四份数据的时间戳对不齐。 */
  const POLL_MS = 3000;

  let data = $state<Overview | null>(null);
  let failure = $state('');
  let lastSync = $state(0);

  async function refresh(): Promise<void> {
    try {
      data = await api.overview();
      lastSync = Date.now();
      failure = '';
    } catch (cause) {
      failure = cause instanceof Error ? cause.message : '概览读取失败';
    }
  }

  $effect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  });

  const liveSeconds = $derived(lastSync ? Math.floor((Date.now() - lastSync) / 1000) : 0);
  /** 柱高按峰值归一：24 桶里有一段尖峰时，其余桶才不会压成一条线。
   *  peak 兜底 1 是给除法用的；peakBytes 是给人看的（没有流量就不该显示「峰值 1 B/h」）。 */
  const peakBytes = $derived(
    data ? Math.max(0, ...data.traffic.interactive, ...data.traffic.other) : 0,
  );
  const peak = $derived(Math.max(1, peakBytes));

  function meter(level: number, cap: number): { width: string; warn: boolean } {
    const ratio = cap > 0 ? level / cap : 0;
    return { width: `${Math.min(100, Math.max(0, ratio * 100))}%`, warn: ratio > 0.6 };
  }

  function disconnect(name: string): void {
    toast(`${name} 已断开`, 'ok');
  }
</script>

<div class="pagehead">
  <h1>概览</h1>
  <span class="live-pill">
    <span class="dot {failure ? 'dot-warn' : 'dot-on'}"></span>
    {#if failure}连接中断{:else}live · {liveSeconds}s{/if}
  </span>
</div>

{#if failure}
  <div
    class="mb-4 rounded-lg border border-warn/45 bg-warn/10 px-3 py-2.5 text-[13px] text-warn"
    role="alert"
  >
    连接中断，正在重试——下方数据可能已过期。
  </div>
{/if}

{#if !data}
  <div class="card p-8 text-center text-sm text-muted">加载中…</div>
{:else}
  <div class="grid grid-cols-[repeat(auto-fit,minmax(190px,1fr))] gap-4">
    <div class="card p-4">
      <div class="text-xs text-muted">
        <span class="dot mr-2 {data.kpi.instancesOnline > 0 ? 'dot-on' : 'dot-off'}"></span>在线实例
      </div>
      <div class="mt-1.5 text-[26px] font-semibold">
        {data.kpi.instancesOnline}<span class="text-sm text-muted">
          / {data.kpi.instancesTotal}</span
        >
      </div>
      {#if data.kpi.instancesOfflineFor != null}
        <div class="mt-1 text-xs text-warn">
          1 台离线 · {relativeTime(Date.now() / 1000 - data.kpi.instancesOfflineFor)}
        </div>
      {/if}
    </div>
    <div class="card p-4">
      <div class="text-xs text-muted">活跃设备 · 1h</div>
      <div class="mt-1.5 text-[26px] font-semibold">{data.kpi.activeDevices}</div>
    </div>
    <div class="card p-4">
      <div class="text-xs text-muted">流量 · 24h</div>
      <div class="mt-1.5 text-[26px] font-semibold">{bytes(data.kpi.bytes24h)}</div>
    </div>
    <div class="card p-4">
      <div class="text-xs text-muted">限流 · 24h</div>
      <div class="mt-1.5 text-[26px] font-semibold">{data.kpi.throttled24h}</div>
      <div class="mt-1 text-xs text-accent">交互 {data.kpi.interactiveThrottled24h} 次</div>
    </div>
  </div>

  <div class="card mt-4 p-4">
    <div
      class="flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5 max-[560px]:flex-col max-[560px]:items-start"
    >
      <b class="text-sm">水位</b>
      <span class="text-xs text-muted"
        >超限时 批量限流 · 页面排队 · <span class="text-accent">交互不断</span></span
      >
    </div>
    <div class="mt-3 grid grid-cols-4 gap-x-8 gap-y-3 max-[900px]:grid-cols-2">
      {#each [{ label: '交互', key: 'interactive' }, { label: '页面', key: 'page' }, { label: '批量', key: 'bulk' }, { label: '带宽', key: 'bandwidth' }] as row (row.key)}
        {@const slot = data.waterline[row.key as keyof typeof data.waterline]}
        {@const bar = meter(slot.level, slot.cap)}
        <div class="gov-row">
          <span class="w-[64px] text-muted">{row.label}</span>
          <div class="meter"><i class:w-warn={bar.warn} style="width:{bar.width}"></i></div>
          <span class="font-mono tabular-nums">
            <!-- 带宽是「桶余量」的百分比，不是字节：M2 没有时间序列的字节语义，
                 而桶快见底这件事必须一眼可见（docs/20 §4.3）。 -->
            {row.key === 'bandwidth' ? `${slot.level}% / 100%` : `${slot.level} / ${slot.cap}`}
          </span>
        </div>
      {/each}
    </div>
  </div>

  <div class="mt-4 grid grid-cols-[2fr_1fr] items-start gap-4 max-[900px]:grid-cols-1">
    <div class="card overflow-hidden">
      <div class="flex items-center justify-between border-b border-line px-4 py-3">
        <b class="text-sm">实例与使用</b>
      </div>
      {#if data.instances.length === 0}
        <div class="px-6 py-10 text-center text-[13px] leading-[1.9] text-muted">
          还没有实例。<br />在千寻桌面端里填入这里的中继地址即可接入。
        </div>
      {:else}
        <table>
          <thead>
            <tr>
              <th class="text-left">实例</th>
              <th class="text-left">版本</th>
              <th class="text-left">连接</th>
              <th class="text-left">设备</th>
              <th class="text-right">流量 · 1h</th>
              <th class="text-right">治理</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {#each data.instances as instance (instance.id)}
              <tr class="hover:bg-surface2/50 {instance.online ? '' : 'row-off'}">
                <td class="wrap">
                  <div class="flex items-center">
                    <span class="dot mr-2 {instance.online ? 'dot-on' : 'dot-off'}"></span>
                    <div>
                      <div class="font-medium">{instance.name}</div>
                      <div class="cell-sub font-mono">{instance.id}</div>
                    </div>
                  </div>
                </td>
                <td class="text-muted">{instance.version ?? '—'}</td>
                <td
                  >{instance.online
                    ? `${Math.floor((instance.connectedFor ?? 0) / 3600)} 小时`
                    : `离线 ${relativeTime(Date.now() / 1000 - (instance.connectedFor ?? 0))}`}</td
                >
                <td class="wrap"
                  >{instance.devices.length > 0 ? instance.devices.join(' · ') : '（出站拉取）'}</td
                >
                <td class="text-right font-mono tabular-nums"
                  >{instance.bytes1h > 0 ? bytes(instance.bytes1h) : '—'}</td
                >
                <td class="text-right">
                  {#if !instance.online}
                    <span class="tag tag-off">离线</span>
                  {:else if instance.status === 'limited'}
                    <span class="tag tag-warn">批量受限</span>
                  {:else}
                    <span class="tag tag-ok">正常</span>
                  {/if}
                </td>
                <td class="text-right">
                  {#if instance.online}
                    <button
                      class="btn btn-danger btn-sm focus-ring"
                      onclick={() => disconnect(instance.name)}>断开</button
                    >
                  {:else}
                    <span class="text-muted">—</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>

    <div class="card p-4">
      <div class="flex items-center justify-between">
        <b class="text-sm">流量 · 24h</b>
        <div class="flex gap-3.5 text-xs text-muted">
          <span><i class="mb-px mr-1.5 inline-block h-2 w-2 rounded-sm bg-accent"></i>交互</span>
          <span><i class="mb-px mr-1.5 inline-block h-2 w-2 rounded-sm bg-info"></i>页面/批量</span>
        </div>
      </div>
      <div class="mt-3 flex h-[190px] items-end gap-1">
        {#each data.traffic.interactive as value, i (i)}
          <span
            class="bar {data.traffic.other[i] && data.traffic.other[i]! > value ? 'bar-b' : ''}"
            style="height:{Math.max(2, (value / peak) * 100)}%"
            title="交互 {bytes(value)} · 页面/批量 {bytes(data.traffic.other[i] ?? 0)}"
          ></span>
        {/each}
      </div>
      <div class="cell-sub mt-1">
        {#if peakBytes > 0}
          峰值 {bytes(peakBytes)}/h · 共 {percent(data.kpi.bytes24h / Math.max(1, peakBytes * 24))} 配额
        {:else}
          暂无流量
        {/if}
      </div>
    </div>
  </div>
{/if}
