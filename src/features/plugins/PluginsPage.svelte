<script lang="ts">
  import { onMount } from 'svelte';
  import { openPath } from '@tauri-apps/plugin-opener';
  import { call } from '../../lib/ipc';
  import type { BridgeStatus, PluginEntry } from '../../lib/ipc/contract';
  import { nav } from '../../stores/nav.svelte';
  import { settings } from '../../stores/settings.svelte';
  import { harness } from '../../stores/harness.svelte';

  let bridge = $state<BridgeStatus | null>(null);
  let plugins = $state<PluginEntry[]>([]);
  let bridgeBusy = $state(false);
  let bridgeError = $state('');
  const vaultReady = $derived((settings.current?.notes.vaultDir ?? '').trim().length > 0);

  onMount(() => {
    void refresh();
  });

  async function refresh(): Promise<void> {
    try {
      bridge = await call<BridgeStatus>('bridge_status');
    } catch {
      bridge = null;
    }
    try {
      plugins = await call<PluginEntry[]>('plugins_list');
    } catch {
      plugins = [];
    }
  }

  async function deployBridge(): Promise<void> {
    bridgeBusy = true;
    bridgeError = '';
    try {
      bridge = await call<BridgeStatus>('bridge_deploy');
      plugins = await call<PluginEntry[]>('plugins_list');
    } catch (error) {
      bridgeError = error instanceof Error ? error.message : String(error);
    } finally {
      bridgeBusy = false;
    }
  }

  /** 部署过但 DSH 在跑：重启才加载。按钮只在「重启有意义」时出现。 */
  const restartMeaningful = $derived(
    bridge !== null && bridge.deployed && bridge.dshRunning && !harness.restarting,
  );

  async function restartDsh(): Promise<void> {
    bridgeError = '';
    try {
      await harness.restart();
      bridge = await call<BridgeStatus>('bridge_status');
    } catch (error) {
      bridgeError = error instanceof Error ? error.message : String(error);
    }
  }

  /** 三项部署事实的状态行；DSH 运行态单独一行（语义不同）。 */
  const bridgeRows = $derived(
    bridge
      ? [
          { ok: bridge.deployed, text: bridge.deployed ? '插件已就位' : '插件未部署' },
          {
            ok: bridge.patchEntry,
            text: bridge.patchEntry ? '装配条目已写入' : '装配条目未写入',
          },
          {
            ok: bridge.vaultMatch,
            text: bridge.vaultMatch ? '笔记库配置一致' : '笔记库配置不一致，重新部署即可',
          },
        ]
      : [],
  );
</script>

<section class="mx-auto max-w-2xl space-y-6">
  <header>
    <h1 class="text-lg font-semibold">插件</h1>
    <p class="mt-1 text-sm text-muted">
      千寻经 DSH 插件系统扩展其行为；插件的部署与生效都在这里完成。
    </p>
  </header>

  <!-- 千寻笔记桥 -->
  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <div class="flex items-start justify-between gap-4">
      <div>
        <h2 class="text-sm font-medium">千寻笔记桥</h2>
        <p class="mt-1 text-xs text-muted">
          把笔记库注入 DSH：agent 可直接检索与读写笔记（note_search / note_read / note_write）。
        </p>
      </div>
      <span class="shrink-0 rounded bg-accent-soft px-1.5 py-0.5 font-mono text-xs text-muted">
        qx-bridge
      </span>
    </div>

    {#if bridge}
      <ul class="space-y-1.5 text-xs">
        {#each bridgeRows as item (item.text)}
          <li class="flex items-center gap-2">
            <span class={item.ok ? 'text-ok' : 'text-danger'}>{item.ok ? '✓' : '✗'}</span>
            {item.text}
          </li>
        {/each}
        <li class="flex items-center gap-2">
          <span class={bridge.dshRunning ? 'text-accent' : 'text-muted'}>
            {bridge.dshRunning ? '⏳' : '○'}
          </span>
          {bridge.dshRunning ? 'DSH 运行中，重启后加载' : 'DSH 未运行，下次启动加载'}
        </li>
      </ul>
      {#if bridge.pluginDir}
        <p class="truncate font-mono text-xs text-muted" title={bridge.pluginDir}>
          {bridge.pluginDir}
        </p>
      {/if}
    {:else}
      <p class="text-xs text-muted">读取状态中…</p>
    {/if}

    <div class="flex flex-wrap items-center gap-2">
      <button
        class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-40"
        disabled={bridgeBusy || !vaultReady}
        onclick={() => void deployBridge()}
      >
        {bridgeBusy ? '部署中…' : '部署 / 修复'}
      </button>
      {#if restartMeaningful}
        <button
          class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft disabled:opacity-40"
          disabled={harness.restarting}
          data-testid="plugin-restart-dsh"
          onclick={() => void restartDsh()}
        >
          {harness.restarting ? '重启中…' : '重启 DSH 生效'}
        </button>
      {/if}
      {#if bridge?.pluginDir}
        <button
          class="rounded-md px-2 py-1.5 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
          onclick={() => void openPath(bridge?.pluginDir ?? '').catch(() => {})}
        >
          打开目录
        </button>
      {/if}
      {#if !vaultReady}
        <button
          class="text-xs text-muted transition-colors hover:text-accent"
          onclick={() => nav.go('notes')}
        >
          请先初始化笔记库 →
        </button>
      {/if}
    </div>
    {#if bridgeError}
      <p class="text-sm text-danger">{bridgeError}</p>
    {/if}
  </section>

  <!-- 已注册插件 -->
  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-medium">已注册插件</h2>
      <button
        class="rounded px-2 py-1 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
        onclick={() => void refresh()}
      >
        刷新
      </button>
    </div>
    {#if plugins.length === 0}
      <p class="text-xs text-muted">
        尚无注册插件。部署笔记桥后，这里会列出 DSH profile 里的全部插件。
      </p>
    {:else}
      <ul class="divide-y divide-line">
        {#each plugins as plugin (plugin.id)}
          <li class="flex items-center gap-3 py-2 text-sm">
            <span class={plugin.deployed ? 'text-ok' : 'text-danger'}>
              {plugin.deployed ? '✓' : '✗'}
            </span>
            <span class="min-w-0 flex-1 truncate text-fg">{plugin.name}</span>
            <span class="shrink-0 font-mono text-xs text-muted">{plugin.id}</span>
            <span class="shrink-0 text-xs {plugin.deployed ? 'text-muted' : 'text-danger'}">
              {plugin.deployed ? '已就位' : '未部署'}
            </span>
          </li>
        {/each}
      </ul>
      <p class="text-xs text-muted">清单来自 DSH profile 的装配配置（cordis.patch.yml）。</p>
    {/if}
  </section>
</section>
