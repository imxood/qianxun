<script lang="ts">
  import { api, type Device } from '../lib/api';
  import { bytes, isoDate, percent, relativeTime } from '../lib/fmt';
  import { toast } from '../lib/toast.svelte';
  import PairDialog from './PairDialog.svelte';

  /** 3 秒轮询一次设备表（docs/20 §4.11）：单端点聚合，行展开的会话数据随列表回来。 */
  const POLL_MS = 3000;

  let devices = $state<Device[]>([]);
  let loaded = $state(false);
  let failure = $state('');
  let lastSync = $state(0);
  let expanded = $state<string | null>(null);
  let pairOpen = $state(false);
  /** 批量吊销后常驻的红条：不可逆，所以没有撤销（docs/20 §4.10）。 */
  let panicked = $state(false);
  /** 轮换后的明文只在这一刻可见：库里只存哈希，关掉就再也取不回（docs/20 §4.5）。 */
  let revealed = $state<{ deviceId: string; name: string; token: string } | null>(null);
  let busy = $state<string | null>(null);

  async function refresh(): Promise<void> {
    try {
      devices = (await api.devices()).devices;
      lastSync = Date.now();
      failure = '';
    } catch (cause) {
      failure = cause instanceof Error ? cause.message : '设备列表读取失败';
    } finally {
      loaded = true;
    }
  }

  $effect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(timer);
  });

  const liveSeconds = $derived(lastSync ? Math.floor((Date.now() - lastSync) / 1000) : 0);

  function shortId(id: string): string {
    return id.startsWith('dev-') ? id.slice(4, 8) : id.slice(0, 6);
  }

  function revoke(device: Device): void {
    busy = device.id;
    api
      .revoke(device.id)
      .then(() => {
        toast(`已吊销 ${device.name}`, 'ok', () => {
          api
            .restore(device.id)
            // 撤销后立刻刷新：等下一轮 3 秒轮询的话，点完「撤销」那一行还会
            // 灰着显示「已吊销」，像是没点生效。
            .then(() => {
              toast(`${device.name} 已恢复`);
              void refresh();
            })
            .catch((cause: unknown) =>
              toast(cause instanceof Error ? cause.message : '恢复失败', 'fail'),
            );
        });
        void refresh();
      })
      .catch((cause: unknown) => toast(cause instanceof Error ? cause.message : '吊销失败', 'fail'))
      .finally(() => (busy = null));
  }

  /** 批量吊销不可逆：直接执行，红条常驻到用户关闭（docs/20 §4.10）。 */
  function revokeAll(): void {
    busy = 'all';
    api
      .revokeAll()
      .then((result) => {
        panicked = result.revoked > 0;
        toast(`已吊销 ${result.revoked} 台设备`);
      })
      .catch((cause: unknown) =>
        toast(cause instanceof Error ? cause.message : '吊销全部失败', 'fail'),
      )
      .finally(() => {
        busy = null;
        void refresh();
      });
  }

  function rotate(device: Device): void {
    busy = device.id;
    api
      .rotate(device.id)
      .then((result) => {
        revealed = { deviceId: device.id, name: device.name, token: result.token };
        toast(`${device.name} 凭据已轮换，旧凭据立即失效`);
      })
      .catch((cause: unknown) => toast(cause instanceof Error ? cause.message : '轮换失败', 'fail'))
      .finally(() => {
        busy = null;
        void refresh();
      });
  }

  async function copyToken(token: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(token);
      toast('凭据已复制');
    } catch {
      // 非安全上下文（局域网 http 访问）没有 clipboard：让人工复制，别静默失败。
      toast('复制失败，请手动选中复制', 'fail');
    }
  }
</script>

<div class="pagehead">
  <div class="flex items-center gap-3">
    <h1>设备</h1>
    <span class="live-pill">
      <span class="dot {failure ? 'dot-warn' : 'dot-on'}"></span>
      {#if failure}连接中断{:else}live · {liveSeconds}s{/if}
    </span>
  </div>
  <div class="flex items-center gap-2.5">
    <button class="btn btn-danger btn-sm focus-ring" disabled={busy === 'all'} onclick={revokeAll}
      >吊销全部</button
    >
    <button class="btn btn-primary btn-sm focus-ring" onclick={() => (pairOpen = true)}
      >＋ 新增设备</button
    >
  </div>
</div>

{#if panicked}
  <div
    class="mb-4 flex items-center gap-3 rounded-xl border border-danger/45 bg-danger/10 px-4 py-2.5 text-[13px]"
  >
    <svg
      width="14"
      height="14"
      class="shrink-0 text-danger"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      ><path d="M12 8v5M12 17h.01" /><path
        d="M10.3 3.9L2.5 17.5A2 2 0 004.2 20.5h15.6a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z"
      /></svg
    >
    <b>已吊销全部设备</b><span class="text-muted">手机需重新配对</span>
    <button class="btn btn-ghost btn-sm focus-ring ml-auto" onclick={() => (panicked = false)}
      >关闭</button
    >
  </div>
{/if}

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
{:else if devices.length === 0}
  <div class="card px-6 py-14 text-center">
    <div class="text-[15px] font-semibold">还没有设备</div>
    <p class="mx-auto mt-2 max-w-[380px] text-[13px] leading-[1.9] text-muted">
      点「新增设备」生成配对码，用手机扫码即可接入。
    </p>
    <button class="btn btn-primary focus-ring mt-5" onclick={() => (pairOpen = true)}
      >新增设备</button
    >
  </div>
{:else}
  <div class="card overflow-hidden">
    <table>
      <thead>
        <tr>
          <th class="text-left">设备</th>
          <th class="text-left">最后活跃</th>
          <th class="text-left">来源 IP</th>
          <th class="text-right">流量 · 24h</th>
          <th class="text-left">占配额</th>
          <th></th>
        </tr>
      </thead>
      <tbody>
        {#each devices as device (device.id)}
          <tr
            class="cursor-pointer hover:bg-surface2/50 {device.revoked ? 'row-off' : ''}"
            onclick={() => (expanded = expanded === device.id ? null : device.id)}
          >
            <td class="wrap">
              <div class="flex items-center">
                <span
                  class="dot mr-2 {device.revoked
                    ? 'dot-off'
                    : relativeTime(device.lastSeenAt) === '刚刚'
                      ? 'dot-on'
                      : 'dot-warn'}"
                ></span>
                <div>
                  <div class="font-medium">{device.name}</div>
                  <div class="cell-sub font-mono">
                    {shortId(device.id)} · {device.revoked ? '已吊销' : '已授权'}
                  </div>
                </div>
              </div>
            </td>
            <td>{device.revoked ? '—' : relativeTime(device.lastSeenAt)}</td>
            <td class="font-mono text-muted">{device.lastIp ?? '—'}</td>
            <td class="text-right font-mono tabular-nums"
              >{device.bytes24h > 0 ? bytes(device.bytes24h) : '—'}</td
            >
            <td>
              {#if device.revoked}
                <span class="text-muted">—</span>
              {:else if device.bytes24h > 0}
                <div class="flex w-[110px] items-center gap-2">
                  <div class="meter">
                    <i
                      class:w-warn={device.share > 0.7}
                      style="width:{Math.max(2, device.share * 100)}%"
                    ></i>
                  </div>
                  <span class="font-mono text-xs tabular-nums text-muted"
                    >{percent(device.share)}</span
                  >
                </div>
              {:else}
                <span class="text-muted">—</span>
              {/if}
            </td>
            <td class="text-right">
              {#if device.revoked}
                <button
                  class="btn btn-ghost btn-sm focus-ring"
                  onclick={(e) => {
                    e.stopPropagation();
                    api.restore(device.id).then(() => {
                      toast(`${device.name} 已恢复`);
                      void refresh();
                    });
                  }}>恢复</button
                >
              {:else}
                <button
                  class="btn btn-danger btn-sm focus-ring"
                  disabled={busy === device.id}
                  onclick={(e) => {
                    e.stopPropagation();
                    revoke(device);
                  }}>吊销</button
                >
              {/if}
            </td>
          </tr>
          {#if expanded === device.id}
            <tr>
              <td colspan="6" class="bg-bg">
                <div class="grid grid-cols-[1fr_1fr_1fr] gap-4 py-3 max-[900px]:grid-cols-1">
                  <div>
                    <div class="cell-sub mb-2">当前会话</div>
                    {#if device.sessions.length === 0}
                      <div class="text-[13px] text-muted">无</div>
                    {:else}
                      <div class="text-[13px] leading-[1.9]">
                        {#each device.sessions as session (session.instanceId + session.class)}
                          <div>
                            → {session.instanceName} · {session.class === 'interactive'
                              ? '交互'
                              : session.class === 'page'
                                ? '页面'
                                : '批量'}
                            <span class="font-mono text-muted"
                              >{relativeTime(Date.now() / 1000 - session.seconds)} · {bytes(
                                session.bytesPerSec,
                              )}/s</span
                            >
                          </div>
                        {/each}
                      </div>
                    {/if}
                  </div>
                  <div>
                    <div class="cell-sub mb-2">24h 流量</div>
                    <div class="flex h-[46px] items-end gap-[3px]">
                      {#each device.shape as value, i (i)}
                        <span
                          class="bar {i % 3 === 2 ? 'bar-b' : ''}"
                          style="height:{Math.max(3, value * 100)}%"
                        ></span>
                      {/each}
                    </div>
                    <div class="cell-sub mt-1">
                      {device.bytes24h > 0
                        ? `峰值 ${bytes(Math.max(...device.shape))}/h`
                        : '暂无流量'}
                    </div>
                  </div>
                  <div>
                    <div class="cell-sub mb-2">凭据</div>
                    <div class="text-[13px] leading-[1.9]">
                      <div><span class="font-mono text-muted">{device.credential.hint}</span></div>
                      {#if device.credential.issuedAt}
                        <div>
                          签发 {isoDate(device.credential.issuedAt)}{device.credential.expiresAt
                            ? ` · 有效期至 ${isoDate(device.credential.expiresAt)}`
                            : ''}
                        </div>
                      {/if}
                      {#if revealed && revealed.deviceId === device.id}
                        <div class="mt-2 rounded-lg border border-warn/40 bg-warn/10 p-2.5">
                          <div class="text-[12px] text-warn">
                            {revealed.name} 的新凭据（只显示这一次，关闭后无法再取）
                          </div>
                          <div class="mt-1.5 flex items-center gap-2">
                            <code
                              class="min-w-0 flex-1 truncate rounded bg-black/30 px-2 py-1 font-mono text-[12px] text-fg"
                              >{revealed.token}</code
                            >
                            <button
                              class="btn btn-ghost btn-sm focus-ring shrink-0"
                              onclick={() => copyToken(revealed!.token)}>复制</button
                            >
                            <button
                              class="btn btn-ghost btn-sm focus-ring shrink-0"
                              onclick={() => (revealed = null)}>关闭</button
                            >
                          </div>
                        </div>
                      {:else if !device.revoked}
                        <div class="mt-2 flex gap-2">
                          <button
                            class="btn btn-ghost btn-sm focus-ring"
                            disabled={busy === device.id}
                            onclick={() => rotate(device)}>轮换凭据</button
                          >
                        </div>
                      {/if}
                    </div>
                  </div>
                </div>
              </td>
            </tr>
          {/if}
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<PairDialog bind:open={pairOpen} onPaired={() => void refresh()} />
