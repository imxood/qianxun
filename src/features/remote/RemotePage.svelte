<script lang="ts">
  /**
   * 远程页（一级入口，设计 §5.2）：启用开关 / 网卡（EasyTier ⚡ 置顶）/
   * 端口 / 状态行 / 配对（二维码 + 复制链接）/ 设备列表（删除需确认）/
   * 自检（本机带 token 走 /qx-gate，「能不能用」一眼可见）。
   */
  import { onMount } from 'svelte';
  import QRCode from 'qrcode';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { call } from '../../lib/ipc';
  import type { NetInterface, RemoteDevice, RemoteStatus, SelfCheck } from '../../lib/ipc/contract';
  import { harness } from '../../stores/harness.svelte';
  import { settings } from '../../stores/settings.svelte';
  import ConfirmDialog from '../../components/ConfirmDialog.svelte';
  import PromptDialog from '../../components/PromptDialog.svelte';
  import Switch from '../../components/Switch.svelte';

  let status = $state<RemoteStatus | null>(null);
  let interfaces = $state<NetInterface[]>([]);
  let bindIp = $state('');
  let portInput = $state('');
  let error = $state('');
  let selfCheck = $state<SelfCheck | null>(null);
  let checking = $state(false);

  let pairDialogOpen = $state(false);
  let pairUrl = $state('');
  let pairName = $state('');
  let pairCanvas: HTMLCanvasElement | null = $state(null);
  let revokeTarget: RemoteDevice | null = $state(null);

  // 设备按未删在前 + 配对时间倒序排。
  const devices = $derived(
    [...(settings.current?.remote.devices ?? [])].sort(
      (a, b) => Number(a.revoked) - Number(b.revoked) || b.createdAt - a.createdAt,
    ),
  );
  /** DSH 启动时的完整 URL（含 `?token=`），用来在系统浏览器打开。 */
  const dshUrl = $derived(harness.status.phase === 'ready' ? harness.status.url : '');
  const easytierAbsent = $derived(
    interfaces.length > 0 && !interfaces.some((item) => item.easytier),
  );
  const portValid = $derived(
    /^\d+$/.test(portInput) && Number(portInput) >= 1024 && Number(portInput) <= 65535,
  );

  onMount(() => {
    void harness.wire();
    void refresh();
    void loadInterfaces();
  });

  // 设置到达后回显（一次性；之后由控件事件维护草稿）。
  $effect(() => {
    if (settings.current && !portInput) {
      bindIp = settings.current.remote.bindIp;
      portInput = String(settings.current.remote.port);
    }
  });

  $effect(() => {
    if (pairUrl && pairCanvas) {
      void QRCode.toCanvas(pairCanvas, pairUrl, { width: 180 });
    }
  });

  async function refresh(): Promise<void> {
    try {
      status = await call<RemoteStatus>('remote_status');
    } catch {
      status = null;
    }
  }

  async function loadInterfaces(): Promise<void> {
    try {
      interfaces = await call<NetInterface[]>('remote_interfaces');
    } catch {
      interfaces = [];
    }
  }

  /** 任何配置变化即时保存：settings_save 自带 remote sync（指纹判定启停）。 */
  async function apply(patch: {
    enabled?: boolean;
    bindIp?: string;
    port?: number;
  }): Promise<void> {
    error = '';
    try {
      await settings.update({ remote: patch });
      await refresh();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function setBindIp(value: string): void {
    bindIp = value;
    void apply({ bindIp: value });
  }

  function setPort(): void {
    if (portValid) void apply({ port: Number(portInput) });
  }

  function toggleEnabled(value: boolean): void {
    void apply({ enabled: value });
  }

  /** 配对：设备记录 + 二维码 URL。Rust 侧 pair 自带 sync（新 token 即时生效）。 */
  async function pair(name: string): Promise<void> {
    error = '';
    pairDialogOpen = false;
    try {
      pairName = name;
      pairUrl = await call<string>('remote_pair', { name });
      await refresh();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function closePair(): void {
    pairUrl = '';
    pairName = '';
  }

  async function confirmRevoke(): Promise<void> {
    const target = revokeTarget;
    revokeTarget = null;
    if (!target) return;
    error = '';
    try {
      await call('remote_revoke', { id: target.id });
      await refresh();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function runSelfCheck(): Promise<void> {
    checking = true;
    selfCheck = null;
    try {
      selfCheck = await call<SelfCheck>('remote_self_check');
    } catch (cause) {
      selfCheck = {
        ok: false,
        detail: cause instanceof Error ? cause.message : String(cause),
        latencyMs: 0,
      };
    } finally {
      checking = false;
    }
  }

  /** 在系统浏览器打开本机 DSH Web（带启动 token）。 */
  function openDsh(): void {
    if (!dshUrl) return;
    void openUrl(dshUrl).catch((cause: unknown) => {
      error = cause instanceof Error ? cause.message : String(cause);
    });
  }

  async function copyPairLink(): Promise<void> {
    try {
      await navigator.clipboard.writeText(pairUrl);
    } catch {
      // 复制失败不打断：二维码仍可扫。
    }
  }

  function formatDate(ms: number): string {
    const date = new Date(ms);
    const pad = (value: number): string => String(value).padStart(2, '0');
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  }
</script>

<svelte:window onfocus={() => void refresh()} />

<section class="mx-auto w-full max-w-2xl space-y-4">
  <h1 class="text-lg font-semibold">远程</h1>
  <p class="text-xs text-muted">
    手机经 EasyTier 虚拟网访问本机 DSH：千寻开一个网关做唯一入口，配对一次、扫码即用。
  </p>

  {#if easytierAbsent}
    <div class="rounded-lg border border-line bg-card p-3 text-xs">
      <p class="font-medium text-fg">未检测到 EasyTier 虚拟网卡</p>
      <p class="mt-1 leading-5 text-muted">
        请先安装并启动 EasyTier 组网（本机与手机加入同一虚拟网）。启动后其网卡会出现在下方列表，
        名称含 “EasyTier”，选择它即可。
      </p>
    </div>
  {/if}

  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <h2 class="text-sm font-medium">网关</h2>
    <label class="flex items-center justify-between text-sm">
      <span>启用远程访问</span>
      <Switch
        label="启用远程访问"
        checked={settings.current?.remote.enabled ?? false}
        onchange={toggleEnabled}
      />
    </label>
    <label class="flex items-center justify-between gap-4 text-sm">
      <span class="shrink-0">绑定网卡</span>
      <select
        class="min-w-0 flex-1 rounded-md border border-line bg-surface px-2 py-1.5 text-sm disabled:opacity-50"
        disabled={!interfaces.length}
        value={bindIp}
        onchange={(event) => setBindIp(event.currentTarget.value)}
      >
        <option value="" disabled hidden>选择网卡…</option>
        {#each interfaces as item (item.ip)}
          <option value={item.ip}>
            {item.easytier ? '⚡ ' : ''}{item.name}（{item.ip}）
          </option>
        {/each}
      </select>
    </label>
    <label class="flex items-center justify-between gap-4 text-sm">
      <span class="shrink-0">端口</span>
      <input
        class="w-32 rounded-md border border-line bg-surface px-2 py-1 text-right invalid:border-danger"
        type="text"
        inputmode="numeric"
        bind:value={portInput}
        onblur={setPort}
        onkeydown={(event) => {
          if (event.key === 'Enter') setPort();
        }}
      />
    </label>
    {#if !portValid}
      <p class="text-xs text-danger">端口范围 1024–65535</p>
    {/if}
    <div class="flex h-5 items-center gap-2 text-xs">
      {#if error}
        <span class="text-danger">{error}</span>
      {:else if status}
        {#if status.listening}
          <span class="rounded-full bg-accent-soft px-2 py-0.5 text-muted">
            监听 {status.listening}
          </span>
        {:else}
          <span class="text-muted">未监听</span>
        {/if}
        <span class="text-muted">
          DSH {status.dshRunning ? '运行中' : '未运行（网关已就绪，DSH 起来即通）'}
        </span>
        <span class="text-muted">设备 {status.activeCount}/{status.deviceCount}</span>
        {#if dshUrl}
          <button
            class="shrink-0 rounded px-1.5 py-0.5 text-muted transition-colors hover:bg-accent-soft hover:text-fg"
            title="在系统浏览器打开 DSH Web 界面（带启动 token）"
            onclick={openDsh}
          >
            打开 DSH Web
          </button>
        {/if}
      {:else}
        <span class="text-muted">状态加载中…</span>
      {/if}
    </div>
  </section>

  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-medium">配对设备</h2>
      <button
        class="rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-accent/90"
        onclick={() => (pairDialogOpen = true)}
      >
        配对新设备
      </button>
    </div>

    {#if pairUrl}
      <div class="flex items-center gap-4 rounded-lg border border-line bg-surface p-3">
        <canvas bind:this={pairCanvas} class="size-44 shrink-0 rounded bg-white p-1"></canvas>
        <div class="min-w-0 flex-1 space-y-2">
          <p class="text-sm text-fg">{pairName}</p>
          <p class="break-all font-mono text-xs text-muted">{pairUrl}</p>
          <div class="flex gap-2">
            <button
              class="rounded-md border border-line px-2.5 py-1 text-xs transition-colors hover:bg-accent-soft"
              onclick={() => void copyPairLink()}
            >
              复制链接
            </button>
            <button
              class="rounded-md px-2.5 py-1 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
              onclick={closePair}
            >
              完成
            </button>
          </div>
          <p class="text-xs text-muted">手机浏览器打开此链接（或扫码）即完成配对。</p>
        </div>
      </div>
    {/if}

    {#if devices.length === 0}
      <p class="text-xs text-muted">还没有设备。点击「配对新设备」生成二维码。</p>
    {:else}
      <ul class="divide-y divide-line/60">
        {#each devices as device (device.id)}
          <li class="flex items-center gap-3 py-2 text-sm">
            <span
              class="min-w-0 flex-1 truncate {device.revoked
                ? 'text-muted line-through'
                : 'text-fg'}"
            >
              {device.name}
            </span>
            <span class="shrink-0 text-xs text-muted">{formatDate(device.createdAt)}</span>
            {#if device.revoked}
              <span class="shrink-0 rounded bg-surface px-1.5 py-0.5 text-xs text-muted"
                >已删除</span
              >
            {:else}
              <button
                class="shrink-0 rounded px-1.5 py-0.5 text-xs text-danger transition-colors hover:bg-danger/10"
                onclick={() => (revokeTarget = device)}
              >
                删除
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="space-y-2 rounded-lg border border-line bg-card p-4">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-medium">自检</h2>
      <button
        class="rounded-md border border-line px-3 py-1.5 text-xs transition-colors hover:bg-accent-soft disabled:opacity-50"
        disabled={checking}
        onclick={() => void runSelfCheck()}
      >
        {checking ? '检查中…' : '检查网关'}
      </button>
    </div>
    {#if selfCheck}
      <p class="text-xs {selfCheck.ok ? 'text-fg' : 'text-danger'}">
        {selfCheck.ok ? '✓ ' : '✗ '}{selfCheck.detail}
      </p>
    {:else}
      <p class="text-xs text-muted">本机带设备 token 请求一次配对入口，验证网关端到端可用。</p>
    {/if}
  </section>
</section>

<PromptDialog
  open={pairDialogOpen}
  title="配对新设备"
  label="设备名（如：我的手机）"
  placeholder="我的手机"
  confirmLabel="生成二维码"
  onconfirm={(value) => void pair(value)}
  oncancel={() => (pairDialogOpen = false)}
/>
<ConfirmDialog
  open={revokeTarget !== null}
  title="删除设备"
  message="删除后该设备的已登录会话将立即断开，需要重新配对才能再次访问。确定删除「{revokeTarget?.name ??
    ''}」？"
  confirmLabel="删除"
  danger
  onconfirm={() => void confirmRevoke()}
  oncancel={() => (revokeTarget = null)}
/>
