<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from '../../lib/ipc';
  import type {
    AppMetaResult,
    BridgeStatus,
    SyncStatus,
    ThemePreference,
  } from '../../lib/ipc/contract';
  import { nav } from '../../stores/nav.svelte';
  import { settings } from '../../stores/settings.svelte';
  import { theme } from '../../stores/theme.svelte';
  import Switch from '../../components/Switch.svelte';

  let meta: AppMetaResult | null = $state(null);
  let saveError: string | null = $state(null);
  let portInput: string = $state('');
  let registryInput: string = $state('');
  /** 千寻锁定的 DSH 安装说明符（Rust 侧单一事实源）。 */
  let installSpec: string | null = $state(null);

  const themeOptions: Array<{ value: ThemePreference; label: string }> = [
    { value: 'system', label: '跟随系统' },
    { value: 'light', label: '浅色' },
    { value: 'dark', label: '深色' },
  ];

  // 端口合法区间：避开系统保留段，也避开 0（那是动态端口，与 ADR-002 相悖）。
  const portValid = $derived(
    /^[1-9][0-9]{2,4}$/.test(portInput) && Number(portInput) >= 1024 && Number(portInput) <= 65535,
  );
  // 端口 10000 常被本机其他 DSH 实例占用：合法但强烈不建议。
  const portClashesStudio = $derived(portValid && Number(portInput) === 10000);

  const registryValid = $derived(
    ['official', 'npmmirror'].includes(registryInput) || /^https?:\/\/.+/.test(registryInput),
  );

  onMount(() => {
    void (async () => {
      try {
        meta = await call<AppMetaResult>('app_meta');
      } catch {
        // 版本获取失败不影响设置编辑，状态栏已单独展示该错误。
        meta = null;
      }
      try {
        installSpec = (await call<{ installSpec: string }>('harness_environment')).installSpec;
      } catch {
        installSpec = null;
      }
      await refreshSync();
    })();
  });

  // 设置加载完成后把输入框回显（一次性同步，之后由输入事件维护）。
  $effect(() => {
    if (settings.current && !portInput) {
      portInput = String(settings.current.dsh.port);
      registryInput = settings.current.mirrors.npmRegistry;
    }
  });

  async function save(patch: Parameters<typeof settings.update>[0]): Promise<void> {
    try {
      saveError = null;
      await settings.update(patch);
    } catch (error) {
      saveError = error instanceof Error ? error.message : String(error);
    }
  }

  function onTheme(value: ThemePreference): void {
    theme.set(value);
    void save({ theme: value });
  }

  // ---- 截屏热键（录制式输入框）----
  let hotkeyRecording = $state(false);
  let hotkeyDraft = $state('');
  let hotkeyError = $state('');

  $effect(() => {
    if (settings.current && !hotkeyDraft && !hotkeyRecording) {
      hotkeyDraft = settings.current.hotkeys.screenshot;
    }
  });

  /** 把键盘事件翻译成 Tauri 快捷键语法（"Ctrl+Alt+A" 形态）。 */
  function accelFrom(event: KeyboardEvent): string | null {
    if (['Control', 'Shift', 'Alt', 'Meta'].includes(event.key)) return null;
    const parts: string[] = [];
    if (event.ctrlKey || event.metaKey) parts.push('Ctrl');
    if (event.altKey) parts.push('Alt');
    if (event.shiftKey) parts.push('Shift');
    const key = event.key.length === 1 ? event.key.toUpperCase() : event.key;
    parts.push(key);
    const accel = parts.join('+');
    // 至少一个修饰键 + 一个普通键，避免吞掉单键输入。
    if (parts.length < 2) return null;
    return accel;
  }

  function onHotkeyKeydown(event: KeyboardEvent): void {
    event.preventDefault();
    if (event.key === 'Escape') {
      hotkeyRecording = false;
      return;
    }
    const accel = accelFrom(event);
    if (accel) {
      hotkeyDraft = accel;
      hotkeyRecording = false;
    }
  }

  async function applyHotkey(): Promise<void> {
    hotkeyError = '';
    try {
      const accel = hotkeyDraft.trim();
      if (accel) {
        // 先试注册（占用/非法立即报错），成功再落设置。
        await call('shots_set_hotkey', { accel });
      } else {
        await call('shots_clear_hotkey');
      }
      await settings.update({ hotkeys: { screenshot: accel } });
    } catch (error) {
      hotkeyError = error instanceof Error ? error.message : String(error);
    }
  }

  // ---- 终端偏好 ----
  let terminalShell = $state('auto');
  let terminalFontInput = $state(13);
  let terminalScrollInput = $state(5000);
  let terminalError = $state('');

  $effect(() => {
    if (settings.current) {
      terminalShell = settings.current.terminal.shell;
      terminalFontInput = settings.current.terminal.fontSize;
      terminalScrollInput = settings.current.terminal.scrollback;
    }
  });

  async function applyTerminal(): Promise<void> {
    terminalError = '';
    try {
      await settings.update({
        terminal: {
          shell: terminalShell,
          fontSize: Number(terminalFontInput),
          scrollback: Number(terminalScrollInput),
        },
      });
    } catch (error) {
      terminalError = error instanceof Error ? error.message : String(error);
    }
  }

  // ---- DSH 笔记桥（M6） ----
  let bridge = $state<BridgeStatus | null>(null);
  let bridgeBusy = $state(false);
  let bridgeError = $state('');
  const vaultReady = $derived((settings.current?.notes.vaultDir ?? '').trim().length > 0);

  $effect(() => {
    if (settings.current) void refreshBridge();
  });

  async function refreshBridge(): Promise<void> {
    try {
      bridge = await call<BridgeStatus>('bridge_status');
    } catch {
      bridge = null;
    }
  }

  async function deployBridge(): Promise<void> {
    bridgeBusy = true;
    bridgeError = '';
    try {
      bridge = await call<BridgeStatus>('bridge_deploy');
    } catch (error) {
      bridgeError = error instanceof Error ? error.message : String(error);
    } finally {
      bridgeBusy = false;
    }
  }

  // ---- 同步（S1 第一阶段：vault 走 git） ----
  let syncStatus = $state<SyncStatus | null>(null);
  let syncBusy = $state(false);
  let syncError = $state('');
  let syncLog = $state<string[]>([]);

  async function refreshSync(): Promise<void> {
    try {
      syncStatus = await call<SyncStatus>('sync_status');
    } catch (error) {
      syncError = error instanceof Error ? error.message : String(error);
    }
  }

  async function syncAction(command: 'sync_init' | 'sync_pull' | 'sync_push'): Promise<void> {
    syncBusy = true;
    syncError = '';
    syncLog = [];
    try {
      syncLog = (await call<string[]>(command)) ?? [];
      await refreshSync();
    } catch (error) {
      syncError = error instanceof Error ? error.message : String(error);
    } finally {
      syncBusy = false;
    }
  }
</script>

<section class="mx-auto max-w-2xl space-y-6">
  <h1 class="text-lg font-semibold">设置</h1>

  {#if settings.loadError}
    <div class="rounded-lg border border-danger bg-danger/10 p-4 text-sm">
      <p class="font-medium">设置加载失败</p>
      <p class="mt-1 text-muted">{settings.loadError}</p>
      <button class="mt-2 text-accent hover:underline" onclick={() => void settings.load()}>
        重试
      </button>
    </div>
  {:else if !settings.current}
    <p class="text-sm text-muted">正在加载设置…</p>
  {:else}
    <section class="space-y-3 rounded-lg border border-line bg-card p-4">
      <h2 class="text-sm font-medium">外观</h2>
      <div class="flex gap-2">
        {#each themeOptions as option (option.value)}
          <button
            class="rounded-md border px-3 py-1.5 text-sm transition-colors {theme.preference ===
            option.value
              ? 'border-accent bg-accent-soft text-fg'
              : 'border-line text-muted hover:text-fg'}"
            onclick={() => onTheme(option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </section>

    <section class="space-y-3 rounded-lg border border-line bg-card p-4">
      <h2 class="text-sm font-medium">窗口行为</h2>
      <label class="flex items-center justify-between text-sm">
        <span>关闭窗口时隐藏到托盘</span>
        <Switch
          label="关闭时隐藏到托盘"
          checked={settings.current.window.closeToTray}
          onchange={(value) => void save({ window: { closeToTray: value } })}
        />
      </label>
      <label class="flex items-center justify-between text-sm">
        <span>启动时最小化到托盘</span>
        <Switch
          label="启动时最小化"
          checked={settings.current.window.startMinimized}
          onchange={(value) => void save({ window: { startMinimized: value } })}
        />
      </label>
    </section>

    <section class="space-y-3 rounded-lg border border-line bg-card p-4">
      <h2 class="text-sm font-medium">DSH</h2>
      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">固定端口</span>
        <input
          class="w-32 rounded-md border border-line bg-surface px-2 py-1 text-right invalid:border-danger"
          type="text"
          inputmode="numeric"
          bind:value={portInput}
          onblur={() => {
            if (portValid) void save({ dsh: { port: Number(portInput) } });
          }}
        />
      </div>
      <p class="text-xs {portValid ? 'text-muted' : 'text-danger'}">
        {portValid ? '1024–65535' : '范围 1024–65535'}
      </p>
      {#if portClashesStudio}
        <p class="text-xs text-danger">10000 常被本机其他 DSH 实例占用，建议更换。</p>
      {/if}

      <label class="flex items-center justify-between text-sm">
        <span>随千寻启动 DSH</span>
        <Switch
          label="随千寻启动 DSH"
          checked={settings.current.dsh.autostart}
          onchange={(value) => void save({ dsh: { autostart: value } })}
        />
      </label>

      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">DSH_HOME</span>
        <select
          class="rounded-md border border-line bg-surface px-2 py-1"
          value={settings.current.dsh.home}
          onchange={(event) => {
            const value = event.currentTarget.value as 'isolated' | 'system';
            void save({ dsh: { home: value } });
          }}
        >
          <option value="isolated">隔离（推荐）</option>
          <option value="system">系统 ~/.dsh</option>
        </select>
      </div>

      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">锁定版本</span>
        <span class="truncate font-mono text-xs text-muted" title="由千寻版本锁定，不可修改">
          {installSpec ?? '…'}
        </span>
      </div>
      <p class="text-xs text-muted">
        DSH 版本由千寻锁定并经过验收，随千寻更新而升级；本页不可修改。
      </p>

      <label class="flex items-center justify-between text-sm">
        <span>端口占用时改用随机端口</span>
        <Switch
          label="允许随机端口回退"
          checked={settings.current.dsh.allowRandomFallback}
          onchange={(value) => void save({ dsh: { allowRandomFallback: value } })}
        />
      </label>
    </section>

    <section class="space-y-3 rounded-lg border border-line bg-card p-4">
      <h2 class="text-sm font-medium">镜像源</h2>
      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">Node 下载源</span>
        <select
          class="rounded-md border border-line bg-surface px-2 py-1"
          value={settings.current.mirrors.nodeBinary}
          onchange={(event) => {
            const value = event.currentTarget.value as 'auto' | 'official' | 'npmmirror';
            void save({ mirrors: { nodeBinary: value } });
          }}
        >
          <option value="auto">自动（官方优先，失败转 npmmirror）</option>
          <option value="official">仅官方</option>
          <option value="npmmirror">仅 npmmirror</option>
        </select>
      </div>
      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">npm registry</span>
        <input
          class="w-56 rounded-md border border-line bg-surface px-2 py-1 invalid:border-danger"
          type="text"
          list="registry-presets"
          bind:value={registryInput}
          onblur={() => {
            if (registryValid) void save({ mirrors: { npmRegistry: registryInput.trim() } });
          }}
        />
        <datalist id="registry-presets">
          <option value="npmmirror">npmmirror（淘宝源）</option>
          <option value="official">npm 官方</option>
        </datalist>
      </div>
      <p class="text-xs {registryValid ? 'text-muted' : 'text-danger'}">
        {registryValid
          ? 'npmmirror / official / http(s):// 自定义地址。'
          : '仅限 npmmirror、official 或 http(s):// 地址。'}
      </p>
    </section>
  {/if}

  {#if saveError}
    <p class="text-sm text-danger">保存失败：{saveError}</p>
  {/if}

  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <h2 class="text-sm font-medium">快捷键</h2>
    <div class="flex items-center gap-3">
      <label for="hotkey-screenshot" class="w-28 shrink-0 text-sm text-muted">截屏</label>
      <input
        id="hotkey-screenshot"
        class="w-44 rounded-md border border-line bg-surface px-3 py-1.5 text-center font-mono text-sm focus:outline-none focus:ring-1 focus:ring-accent"
        type="text"
        readonly
        placeholder="点击录制"
        value={hotkeyRecording ? '按下组合键…（Esc 取消）' : hotkeyDraft}
        onclick={() => (hotkeyRecording = true)}
        onkeydown={onHotkeyKeydown}
      />
      <button
        class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90"
        onclick={() => void applyHotkey()}
      >
        应用
      </button>
    </div>
    <p class="text-xs text-muted">修饰键 + 字母/数字；清空并应用为停用。</p>
    {#if hotkeyError}
      <p class="text-sm text-danger">{hotkeyError}</p>
    {/if}
  </section>

  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <h2 class="text-sm font-medium">终端</h2>
    <div class="flex items-center gap-3">
      <label for="term-shell" class="w-28 shrink-0 text-sm text-muted">Shell</label>
      <select
        id="term-shell"
        class="flex-1 rounded-md border border-line bg-surface px-3 py-1.5 text-sm"
        value={terminalShell}
        onchange={(event) => (terminalShell = event.currentTarget.value)}
      >
        <option value="auto">自动（pwsh 优先）</option>
        <option value="pwsh.exe">pwsh</option>
        <option value="powershell.exe">Windows PowerShell</option>
        <option value="cmd.exe">cmd</option>
      </select>
    </div>
    <div class="flex items-center gap-3">
      <label for="term-font" class="w-28 shrink-0 text-sm text-muted">字号</label>
      <input
        id="term-font"
        class="w-24 rounded-md border border-line bg-surface px-3 py-1.5 text-sm"
        type="number"
        min="8"
        max="32"
        bind:value={terminalFontInput}
      />
      <label for="term-scroll" class="ml-4 w-28 shrink-0 text-sm text-muted">回滚行数</label>
      <input
        id="term-scroll"
        class="w-28 rounded-md border border-line bg-surface px-3 py-1.5 text-sm"
        type="number"
        min="100"
        max="100000"
        step="100"
        bind:value={terminalScrollInput}
      />
      <button
        class="ml-auto rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90"
        onclick={() => void applyTerminal()}
      >
        应用
      </button>
    </div>
    <p class="text-xs text-muted">对新建标签生效。</p>
    {#if terminalError}
      <p class="text-sm text-danger">{terminalError}</p>
    {/if}
  </section>

  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <h2 class="text-sm font-medium">DSH 笔记桥</h2>
    <p class="text-xs text-muted">
      把笔记库注入 DSH：agent 可直接检索与读写笔记。变更后需重启 DSH。
    </p>
    {#if bridge}
      <ul class="space-y-1 text-xs">
        <li>{bridge.deployed ? '✓' : '✗'} 插件{bridge.deployed ? '已就位' : '未部署'}</li>
        <li>
          {bridge.patchEntry ? '✓' : '✗'} 装配条目{bridge.patchEntry ? '已写入' : '未写入'}
        </li>
        <li>
          {bridge.vaultMatch ? '✓' : '✗'} 笔记库{bridge.vaultMatch
            ? '配置一致'
            : '配置不一致，重新部署即可'}
        </li>
        <li>{bridge.dshRunning ? '⏳ DSH 运行中，重启后加载' : 'DSH 未运行，下次启动加载'}</li>
      </ul>
      <p class="truncate text-xs text-muted" title={bridge.pluginDir}>{bridge.pluginDir}</p>
    {:else}
      <p class="text-xs text-muted">读取状态中…</p>
    {/if}
    <div class="flex items-center gap-2">
      <button
        class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-40"
        disabled={bridgeBusy || !vaultReady}
        onclick={() => void deployBridge()}
      >
        {bridgeBusy ? '部署中…' : '部署 / 修复'}
      </button>
      {#if !vaultReady}
        <span class="text-xs text-muted">请先初始化笔记库</span>
      {/if}
      {#if bridgeError}<span class="text-sm text-danger">{bridgeError}</span>{/if}
    </div>
  </section>

  <section class="space-y-2 rounded-lg border border-line bg-card p-4">
    <div class="flex items-center justify-between">
      <h2 class="text-sm font-medium">远程访问</h2>
      <button
        class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
        onclick={() => nav.go('remote')}
      >
        打开远程页
      </button>
    </div>
    <p class="text-xs text-muted">
      远程访问已升格为一级页面：启用网关、配对设备、自检都在「远程」页完成。
    </p>
  </section>

  <section class="space-y-3 rounded-lg border border-line bg-card p-4">
    <h2 class="text-sm font-medium">同步</h2>
    <p class="text-xs text-muted">
      笔记库以 git 同步，仅同步 vault 目录。推送 = 提交并 push；拉取 = rebase。
    </p>
    {#if syncStatus}
      {#if !syncStatus.gitAvailable}
        <p class="text-sm text-danger">未检测到 git，请安装后重试。</p>
      {:else if !syncStatus.initialized}
        <p class="text-sm text-muted">笔记库还不是 git 仓。</p>
        <button
          class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white hover:bg-accent/90 disabled:opacity-40"
          disabled={syncBusy || !vaultReady}
          onclick={() => void syncAction('sync_init')}
        >
          初始化 git 仓
        </button>
      {:else}
        <p class="text-xs text-muted">
          未提交 {syncStatus.dirty} 处
          {#if syncStatus.hasRemote}
            · 领先 {syncStatus.ahead ?? '?'} · 落后 {syncStatus.behind ?? '?'}
          {:else}
            · 无远端
          {/if}
        </p>
        <div class="flex items-center gap-2">
          <button
            class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white hover:bg-accent/90 disabled:opacity-40"
            disabled={syncBusy || !syncStatus.hasRemote}
            title={syncStatus.hasRemote ? '' : '先在终端里给仓加 remote'}
            onclick={() => void syncAction('sync_push')}
          >
            推送
          </button>
          <button
            class="rounded-md border border-line px-3 py-1.5 text-sm hover:bg-accent-soft disabled:opacity-40"
            disabled={syncBusy || !syncStatus.hasRemote}
            onclick={() => void syncAction('sync_pull')}
          >
            拉取
          </button>
          <button
            class="rounded-md px-2 py-1.5 text-xs text-muted hover:bg-accent-soft"
            onclick={() => void refreshSync()}
          >
            刷新
          </button>
        </div>
      {/if}
    {/if}
    {#if syncError}<p class="text-sm text-danger">{syncError}</p>{/if}
    {#if syncLog.length > 0}
      <pre class="max-h-32 overflow-y-auto rounded-md bg-bg p-2 text-xs text-muted">{syncLog.join(
          '\n',
        )}</pre>
    {/if}
  </section>

  <section class="space-y-1 rounded-lg border border-line bg-card p-4 text-xs text-muted">
    <h2 class="text-sm font-medium text-fg">关于</h2>
    {#if meta}
      <p>千寻 v{meta.version} · {meta.identifier}</p>
      <p>Tauri 2 · Svelte 5</p>
    {:else}
      <p>版本信息不可用</p>
    {/if}
  </section>
</section>
