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
  import type {
    NetInterface,
    RemoteDevice,
    RemoteStatus,
    SelfCheck,
    ServerDevice,
    ServerPairOffer,
    ServerStatus,
  } from '../../lib/ipc/contract';
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

  // ---- 服务器（docs/19：单地址接入，手机经服务端访问本机）----
  let serverStatus = $state<ServerStatus | null>(null);
  let serverUrl = $state('');
  let serverId = $state('');
  let serverToken = $state('');
  let serverError = $state('');
  let serverSaving = $state(false);
  let serverPair = $state<ServerPairOffer | null>(null);
  let serverPairCanvas: HTMLCanvasElement | null = $state(null);
  let serverRevokeName: string | null = $state(null);
  let serverRevokeId = $state('');
  let serverConfigLoaded = $state(false);

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
    void refreshServer();
    const timer = setInterval(() => void refreshServer(), 10_000);
    return () => clearInterval(timer);
  });

  // 设置到达后回显（一次性；之后由控件事件维护草稿）。
  $effect(() => {
    if (settings.current && !portInput) {
      bindIp = settings.current.remote.bindIp;
      portInput = String(settings.current.remote.port);
    }
  });

  // 服务器设置回显（一次性；凭据只在编辑后经 server_configure 保存）。
  $effect(() => {
    if (settings.current && !serverConfigLoaded) {
      serverConfigLoaded = true;
      serverUrl = settings.current.server.url;
      serverId = settings.current.server.instanceId;
      serverToken = settings.current.server.instanceToken;
    }
  });

  $effect(() => {
    if (pairUrl && pairCanvas) {
      void QRCode.toCanvas(pairCanvas, pairUrl, { width: 180 });
    }
  });

  $effect(() => {
    if (serverPair && serverPairCanvas) {
      void QRCode.toCanvas(serverPairCanvas, serverPair.pairUrl, { width: 180 });
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

  // ---- 服务器动作 ----

  async function refreshServer(): Promise<void> {
    try {
      serverStatus = await call<ServerStatus>('server_status');
      if (serverStatus.state === 'online') {
        const devices = await call<ServerDevice[]>('server_device_list');
        serverStatus = { ...serverStatus, devices };
      }
    } catch {
      serverStatus = null;
    }
  }

  async function saveServer(): Promise<void> {
    serverError = '';
    serverSaving = true;
    try {
      await call('server_configure', {
        url: serverUrl,
        instanceId: serverId,
        instanceToken: serverToken,
      });
      await settings.load();
      await refreshServer();
    } catch (cause) {
      serverError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      serverSaving = false;
    }
  }

  async function createServerPair(): Promise<void> {
    serverError = '';
    try {
      serverPair = await call<ServerPairOffer>('server_pair_create');
      await refreshServer();
    } catch (cause) {
      serverError = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async function copyServerPairLink(): Promise<void> {
    if (serverPair) {
      try {
        await navigator.clipboard.writeText(serverPair.pairUrl);
      } catch {
        // 复制失败不打断：二维码仍可扫。
      }
    }
  }

  async function revokeServerDevice(): Promise<void> {
    if (!serverRevokeId) return;
    try {
      await call('server_device_revoke', { deviceId: serverRevokeId });
    } catch (cause) {
      serverError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      serverRevokeId = '';
      serverRevokeName = null;
      await refreshServer();
    }
  }

  const serverStateText: Record<string, string> = {
    off: '未配置',
    connecting: '连接中…',
    online: '已连接',
    backoff: '重连中',
    invalid_config: '配置无效',
  };
  const serverDevices = $derived((serverStatus?.devices ?? []).filter((d) => !d.revoked));
</script>

<svelte:window
  onfocus={() => {
    void refresh();
    void refreshServer();
  }}
/>

<section class="mx-auto w-full max-w-2xl space-y-4">
  <h1 class="qx-page-title">远程</h1>

  {#if easytierAbsent}
    <div class="qx-card p-3 text-xs">
      <p class="font-medium text-fg">未检测到 EasyTier 虚拟网卡</p>
      <p class="mt-1 leading-5 text-muted">
        安装并启动 EasyTier 组网（本机与手机同一虚拟网）后，选择名称含 “EasyTier” 的网卡。
      </p>
    </div>
  {/if}

  <section class="qx-card space-y-3 p-4">
    <h2 class="qx-h">网关</h2>
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
        class="qx-select min-w-0 flex-1"
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
        class="qx-input w-32 py-1 text-right invalid:border-danger"
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
          <span class="qx-badge bg-accent-soft text-accent tabular-nums">
            监听 {status.listening}
          </span>
        {:else}
          <span class="text-muted">未监听</span>
        {/if}
        <span class="text-muted">DSH {status.dshRunning ? '运行中' : '未运行'}</span>
        <span class="text-muted tabular-nums">设备 {status.activeCount}/{status.deviceCount}</span>
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

  <section class="qx-card space-y-3 p-4">
    <div class="flex items-center justify-between">
      <h2 class="qx-h">服务器</h2>
      {#if serverStatus}
        <span
          class="qx-badge {serverStatus.state === 'online'
            ? 'bg-accent-soft text-accent'
            : serverStatus.state === 'invalid_config'
              ? 'text-danger'
              : 'bg-surface text-muted'}"
        >
          {serverStateText[serverStatus.state] ?? serverStatus.state}
        </span>
      {/if}
    </div>
    <p class="text-xs leading-5 text-muted">
      接入千寻服务端（云服务器或局域网内另一台机器）后，手机在任意网络都能访问本机—— 无需 EasyTier
      组网。地址形如 <span class="font-mono">wss://relay.example.com</span> 或
      <span class="font-mono">ws://192.168.1.5:23100</span>。
    </p>
    <label class="block space-y-1 text-sm">
      <span class="text-muted">服务器地址</span>
      <input
        class="qx-input w-full"
        type="text"
        placeholder="wss://relay.example.com"
        bind:value={serverUrl}
        spellcheck="false"
      />
    </label>
    <div class="flex gap-3">
      <label class="min-w-0 flex-1 space-y-1 text-sm">
        <span class="text-muted">实例 ID</span>
        <input class="qx-input w-full" type="text" bind:value={serverId} spellcheck="false" />
      </label>
      <label class="min-w-0 flex-1 space-y-1 text-sm">
        <span class="text-muted">实例凭据</span>
        <input
          class="qx-input w-full font-mono"
          type="password"
          bind:value={serverToken}
          spellcheck="false"
        />
      </label>
    </div>
    <div class="flex items-center gap-2">
      <button
        class="qx-btn qx-btn-primary qx-btn-sm"
        disabled={serverSaving}
        onclick={() => void saveServer()}
      >
        {serverSaving ? '保存中…' : '保存并连接'}
      </button>
      <button
        class="qx-btn qx-btn-outline qx-btn-sm"
        disabled={serverStatus?.state !== 'online'}
        onclick={() => void createServerPair()}
      >
        生成配对码
      </button>
      {#if serverError}
        <span class="min-w-0 flex-1 truncate text-xs text-danger" title={serverError}>
          {serverError}
        </span>
      {:else if serverStatus?.detail}
        <span class="min-w-0 flex-1 truncate text-xs text-muted" title={serverStatus.detail}>
          {serverStatus.detail}
        </span>
      {/if}
    </div>

    {#if serverPair}
      <div class="flex items-center gap-4 rounded-xl border border-line bg-surface p-3">
        <canvas bind:this={serverPairCanvas} class="size-44 shrink-0 rounded bg-white p-1"></canvas>
        <div class="min-w-0 flex-1 space-y-2">
          <p class="font-mono text-lg tracking-widest text-fg">{serverPair.code}</p>
          <p class="break-all font-mono text-xs text-muted">{serverPair.pairUrl}</p>
          <div class="flex gap-2">
            <button
              class="qx-btn qx-btn-outline qx-btn-sm"
              onclick={() => void copyServerPairLink()}
            >
              复制链接
            </button>
            <button class="qx-btn qx-btn-ghost qx-btn-sm" onclick={() => (serverPair = null)}>
              完成
            </button>
          </div>
          <p class="text-xs text-muted">手机扫码或打开链接，输入配对码完成绑定（5 分钟内有效）。</p>
        </div>
      </div>
    {/if}

    {#if serverStatus && serverStatus.state === 'online'}
      <div class="space-y-1">
        <p class="text-xs text-muted">已配对设备（服务器级，所有实例共用）</p>
        {#if serverDevices.length === 0}
          <p class="text-xs text-muted">还没有设备——生成配对码让手机扫码。</p>
        {:else}
          <ul class="divide-y divide-line/60">
            {#each serverDevices as device (device.id)}
              <li class="flex items-center gap-3 py-2 text-sm">
                <span class="min-w-0 flex-1 truncate text-fg">{device.name}</span>
                <span class="shrink-0 text-xs text-muted tabular-nums">
                  {formatDate(device.createdAt * 1000)}
                </span>
                <button
                  class="shrink-0 rounded px-1.5 py-0.5 text-xs text-danger transition-colors hover:bg-danger/10"
                  onclick={() => {
                    serverRevokeId = device.id;
                    serverRevokeName = device.name;
                  }}
                >
                  删除
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  </section>

  <section class="qx-card space-y-3 p-4">
    <div class="flex items-center justify-between">
      <h2 class="qx-h">配对设备</h2>
      <button class="qx-btn qx-btn-primary qx-btn-sm" onclick={() => (pairDialogOpen = true)}>
        配对新设备
      </button>
    </div>

    {#if pairUrl}
      <div class="flex items-center gap-4 rounded-xl border border-line bg-surface p-3">
        <canvas bind:this={pairCanvas} class="size-44 shrink-0 rounded bg-white p-1"></canvas>
        <div class="min-w-0 flex-1 space-y-2">
          <p class="text-sm text-fg">{pairName}</p>
          <p class="break-all font-mono text-xs text-muted">{pairUrl}</p>
          <div class="flex gap-2">
            <button class="qx-btn qx-btn-outline qx-btn-sm" onclick={() => void copyPairLink()}>
              复制链接
            </button>
            <button class="qx-btn qx-btn-ghost qx-btn-sm" onclick={closePair}> 完成 </button>
          </div>
          <p class="text-xs text-muted">扫码或打开链接完成配对。</p>
        </div>
      </div>
    {/if}

    {#if devices.length === 0}
      <p class="text-xs text-muted">还没有设备</p>
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
            <span class="shrink-0 text-xs text-muted tabular-nums"
              >{formatDate(device.createdAt)}</span
            >
            {#if device.revoked}
              <span class="qx-badge bg-surface text-muted">已删除</span>
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

  <section class="qx-card space-y-2 p-4">
    <div class="flex items-center justify-between">
      <h2 class="qx-h">自检</h2>
      <button
        class="qx-btn qx-btn-outline qx-btn-sm"
        disabled={checking}
        onclick={() => void runSelfCheck()}
      >
        {checking ? '检查中…' : '检查网关'}
      </button>
    </div>
    {#if selfCheck}
      <p class="text-xs {selfCheck.ok ? 'text-ok' : 'text-danger'}">
        {selfCheck.ok ? '✓ ' : '✗ '}{selfCheck.detail}
      </p>
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
<ConfirmDialog
  open={serverRevokeName !== null}
  title="删除服务器设备"
  message="删除后该设备在所有实例上的访问都会断开，需重新配对。确定删除「{serverRevokeName ??
    ''}」？"
  confirmLabel="删除"
  danger
  onconfirm={() => void revokeServerDevice()}
  oncancel={() => {
    serverRevokeId = '';
    serverRevokeName = null;
  }}
/>
