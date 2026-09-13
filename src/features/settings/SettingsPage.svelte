<script lang="ts">
  import { onMount } from 'svelte';
  // 注意：本页已有设置补丁的 save()，对话框的 save 起别名避免撞名。
  import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
  import { call } from '../../lib/ipc';
  import type {
    AppMetaResult,
    BackupExportResult,
    BackupManifest,
    BackupRestoreReport,
    HarnessStatus,
    SyncStatus,
    ThemePreference,
  } from '../../lib/ipc/contract';
  import { nav } from '../../stores/nav.svelte';
  import { settings } from '../../stores/settings.svelte';
  import { theme } from '../../stores/theme.svelte';
  import ConfirmDialog from '../../components/ConfirmDialog.svelte';
  import Switch from '../../components/Switch.svelte';

  let meta: AppMetaResult | null = $state(null);
  let saveError: string | null = $state(null);
  let registryInput: string = $state('');
  /** 千寻锁定的 DSH 安装说明符（Rust 侧单一事实源）。 */
  let installSpec: string | null = $state(null);

  const themeOptions: Array<{ value: ThemePreference; label: string }> = [
    { value: 'system', label: '跟随系统' },
    { value: 'light', label: '浅色' },
    { value: 'dark', label: '深色' },
  ];

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
    if (settings.current && !registryInput) {
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

  // ---- 同步（S1 第一阶段：vault 走 git） ----
  let syncStatus = $state<SyncStatus | null>(null);
  let syncBusy = $state(false);
  let syncError = $state('');
  let syncLog = $state<string[]>([]);
  const vaultReady = $derived((settings.current?.notes.vaultDir ?? '').trim().length > 0);

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

  // ---- 数据备份（导出/还原 ~/.qianxun：千寻设置 + DSH 全部用户数据） ----
  let backupBusy = $state(false);
  let exportResult = $state<BackupExportResult | null>(null);
  let backupError = $state('');
  /** 待确认的还原包摘要（非 null = 显示确认框）。 */
  let restoreSummary = $state<BackupManifest | null>(null);
  let restorePath = $state('');
  /** 还原成功后的报告（非 null = 显示重启引导框）。 */
  let restoreReport = $state<BackupRestoreReport | null>(null);
  /** 导出时 DSH 正在运行的一致性提醒。 */
  let exportWarnOpen = $state(false);

  const BACKUP_FILTER = { name: '千寻备份', extensions: ['zip'] };

  const restoreMessage = $derived(
    restoreSummary
      ? `创建于 ${restoreSummary.createdAt} · 千寻 v${restoreSummary.appVersion}` +
          `${restoreSummary.dshVersion ? ` · DSH ${restoreSummary.dshVersion}` : ''}；` +
          `工作区 ${restoreSummary.workspaceCount} 个 · 会话 ${restoreSummary.sessionCount} 个 · ` +
          `文件 ${restoreSummary.fileCount} 个。` +
          '还原将覆盖当前全部千寻设置与 DSH 数据（含 agents 配置与 API Key），' +
          '还原前会自动保存一份当前状态快照；DSH 会被先停止。'
      : '',
  );

  const restartMessage = $derived(
    restoreReport
      ? `已还原 ${restoreReport.restoredFiles} 个文件` +
          `${restoreReport.preRestoreBackup ? `，还原前状态已保存到 ${restoreReport.preRestoreBackup}` : ''}。` +
          '重启千寻以加载还原的数据。'
      : '',
  );

  function formatSize(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  /** DSH 是否在跑（状态查询失败不拦截备份操作）。 */
  async function dshRunning(): Promise<boolean> {
    try {
      const status = await call<HarnessStatus>('harness_status');
      return (
        status.phase === 'ready' || status.phase === 'starting' || status.phase === 'restarting'
      );
    } catch {
      return false;
    }
  }

  async function exportBackup(): Promise<void> {
    if (backupBusy) return;
    backupError = '';
    // 会话是追加写：运行中导出可能备份到半截文件，先提醒再放行。
    if (await dshRunning()) {
      exportWarnOpen = true;
      return;
    }
    await runExport();
  }

  async function runExport(): Promise<void> {
    backupBusy = true;
    try {
      const now = new Date();
      const pad = (value: number): string => String(value).padStart(2, '0');
      const stamp =
        `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}` +
        `-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
      const target = await saveDialog({
        defaultPath: `qianxun-backup-${stamp}.zip`,
        filters: [BACKUP_FILTER],
      });
      if (typeof target === 'string' && target.trim()) {
        exportResult = await call<BackupExportResult>('backup_export', { path: target });
      }
    } catch (error) {
      backupError = error instanceof Error ? error.message : String(error);
    } finally {
      backupBusy = false;
    }
  }

  async function pickRestore(): Promise<void> {
    if (backupBusy) return;
    backupError = '';
    exportResult = null;
    try {
      const picked = await openDialog({ multiple: false, filters: [BACKUP_FILTER] });
      if (typeof picked === 'string' && picked.trim()) {
        restorePath = picked;
        restoreSummary = await call<BackupManifest>('backup_inspect', { path: picked });
      }
    } catch (error) {
      backupError = error instanceof Error ? error.message : String(error);
    }
  }

  async function confirmRestore(): Promise<void> {
    const path = restorePath;
    restoreSummary = null;
    backupBusy = true;
    try {
      // 还原要求 DSH 已停止：在跑就先停（harness_stop 等监督循环退出后才返回）。
      if (await dshRunning()) {
        await call('harness_stop');
      }
      restoreReport = await call<BackupRestoreReport>('backup_restore', { path });
    } catch (error) {
      backupError = error instanceof Error ? error.message : String(error);
    } finally {
      backupBusy = false;
    }
  }

  async function restartNow(): Promise<void> {
    try {
      await call('app_restart');
      // restart 不返回：走到这里说明重启没有发生，提示用户手动重启。
      backupError = '自动重启失败，请手动退出并重新打开千寻。';
    } catch {
      backupError = '自动重启失败，请手动退出并重新打开千寻。';
    }
  }

  /** 暂不重启：也让 UI 立即反映还原结果（Rust 侧已重载设置）。 */
  function dismissRestoreReport(): void {
    restoreReport = null;
    void settings.load();
  }
</script>

<section class="mx-auto max-w-2xl space-y-6">
  <h1 class="qx-page-title">设置</h1>

  {#if settings.loadError}
    <div class="rounded-xl border border-danger bg-danger/10 p-4 text-sm">
      <p class="font-medium">设置加载失败</p>
      <p class="mt-1 text-muted">{settings.loadError}</p>
      <button class="mt-2 text-accent hover:underline" onclick={() => void settings.load()}>
        重试
      </button>
    </div>
  {:else if !settings.current}
    <p class="text-sm text-muted">正在加载设置…</p>
  {:else}
    <section class="qx-card space-y-3 p-4">
      <h2 class="qx-h">外观</h2>
      <div class="flex gap-2">
        {#each themeOptions as option (option.value)}
          <button
            class="qx-btn qx-btn-md {theme.preference === option.value
              ? 'qx-btn-soft'
              : 'qx-btn-outline text-muted'}"
            onclick={() => onTheme(option.value)}
          >
            {option.label}
          </button>
        {/each}
      </div>
    </section>

    <section class="qx-card space-y-3 p-4">
      <h2 class="qx-h">窗口行为</h2>
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

    <section class="qx-card space-y-3 p-4">
      <h2 class="qx-h">DSH</h2>
      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">端口</span>
        <span class="font-mono text-xs text-muted">随机分配</span>
      </div>

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
          class="qx-select"
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
    </section>

    <section class="qx-card space-y-3 p-4">
      <h2 class="qx-h">镜像源</h2>
      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">Node 下载源</span>
        <select
          class="qx-select"
          value={settings.current.mirrors.nodeBinary}
          onchange={(event) => {
            const value = event.currentTarget.value as 'auto' | 'official' | 'npmmirror';
            void save({ mirrors: { nodeBinary: value } });
          }}
        >
          <option value="auto">自动（npmmirror 优先）</option>
          <option value="official">仅官方</option>
          <option value="npmmirror">仅 npmmirror</option>
        </select>
      </div>
      <div class="flex items-center justify-between gap-4 text-sm">
        <span class="shrink-0">npm registry</span>
        <input
          class="qx-input w-56 invalid:border-danger"
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
      {#if !registryValid}
        <p class="text-xs text-danger">仅限 npmmirror、official 或 http(s):// 地址。</p>
      {/if}
    </section>
  {/if}

  {#if saveError}
    <p class="text-sm text-danger">保存失败：{saveError}</p>
  {/if}

  <section class="qx-card space-y-3 p-4">
    <h2 class="qx-h">快捷键</h2>
    <div class="flex items-center gap-3">
      <label for="hotkey-screenshot" class="w-28 shrink-0 text-sm text-muted">截屏</label>
      <input
        id="hotkey-screenshot"
        class="qx-input w-44 text-center font-mono"
        type="text"
        readonly
        placeholder="点击录制"
        value={hotkeyRecording ? '按下组合键…（Esc 取消）' : hotkeyDraft}
        onclick={() => (hotkeyRecording = true)}
        onkeydown={onHotkeyKeydown}
      />
      <button class="qx-btn qx-btn-primary qx-btn-md" onclick={() => void applyHotkey()}>
        应用
      </button>
    </div>
    <p class="text-xs text-muted">清空并应用 = 停用。</p>
    {#if hotkeyError}
      <p class="text-sm text-danger">{hotkeyError}</p>
    {/if}
  </section>

  <section class="qx-card space-y-3 p-4">
    <h2 class="qx-h">终端</h2>
    <div class="flex items-center gap-3">
      <label for="term-shell" class="w-28 shrink-0 text-sm text-muted">Shell</label>
      <select
        id="term-shell"
        class="qx-select flex-1"
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
        class="qx-input w-24"
        type="number"
        min="8"
        max="32"
        bind:value={terminalFontInput}
      />
      <label for="term-scroll" class="ml-4 w-28 shrink-0 text-sm text-muted">回滚行数</label>
      <input
        id="term-scroll"
        class="qx-input w-28"
        type="number"
        min="100"
        max="100000"
        step="100"
        bind:value={terminalScrollInput}
      />
      <button class="qx-btn qx-btn-primary qx-btn-md ml-auto" onclick={() => void applyTerminal()}>
        应用
      </button>
    </div>
    <p class="text-xs text-muted">对新建标签生效。</p>
    {#if terminalError}
      <p class="text-sm text-danger">{terminalError}</p>
    {/if}
  </section>

  <section class="qx-card flex items-center justify-between p-4">
    <h2 class="qx-h">远程访问</h2>
    <button class="qx-btn qx-btn-outline qx-btn-md" onclick={() => nav.go('remote')}>
      打开远程页
    </button>
  </section>

  <section class="qx-card space-y-3 p-4">
    <h2 class="qx-h">同步</h2>
    {#if syncStatus}
      {#if !syncStatus.gitAvailable}
        <p class="text-sm text-danger">未检测到 git，请安装后重试。</p>
      {:else if !syncStatus.initialized}
        <p class="text-sm text-muted">笔记库还不是 git 仓。</p>
        <button
          class="qx-btn qx-btn-primary qx-btn-md"
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
            class="qx-btn qx-btn-primary qx-btn-md"
            disabled={syncBusy || !syncStatus.hasRemote}
            title={syncStatus.hasRemote ? '' : '先在终端里给仓加 remote'}
            onclick={() => void syncAction('sync_push')}
          >
            推送
          </button>
          <button
            class="qx-btn qx-btn-outline qx-btn-md"
            disabled={syncBusy || !syncStatus.hasRemote}
            onclick={() => void syncAction('sync_pull')}
          >
            拉取
          </button>
          <button class="qx-btn qx-btn-ghost qx-btn-sm" onclick={() => void refreshSync()}>
            刷新
          </button>
        </div>
      {/if}
    {/if}
    {#if syncError}<p class="text-sm text-danger">{syncError}</p>{/if}
    {#if syncLog.length > 0}
      <pre
        class="max-h-32 overflow-y-auto rounded-md bg-bg p-2 font-mono text-xs text-muted">{syncLog.join(
          '\n',
        )}</pre>
    {/if}
  </section>

  <section class="qx-card space-y-3 p-4">
    <h2 class="qx-h">数据备份</h2>
    <p class="text-xs text-muted">
      打包千寻设置与 DSH 用户数据（含 API Key）为 zip，可随时还原；不含程序本体。
    </p>
    <p class="text-xs text-danger">备份包含 API Key 明文，请妥善保管备份文件。</p>
    <div class="flex items-center gap-2">
      <button
        class="qx-btn qx-btn-primary qx-btn-md"
        disabled={backupBusy}
        onclick={() => void exportBackup()}
      >
        导出备份…
      </button>
      <button
        class="qx-btn qx-btn-outline qx-btn-md"
        disabled={backupBusy}
        onclick={() => void pickRestore()}
      >
        还原备份…
      </button>
    </div>
    {#if exportResult}
      <p class="text-xs break-all text-muted">
        已导出：{exportResult.path}（{formatSize(exportResult.sizeBytes)} ·
        {exportResult.fileCount} 个文件）
      </p>
    {/if}
    {#if backupError}
      <p class="text-sm text-danger">{backupError}</p>
    {/if}
  </section>

  <ConfirmDialog
    open={exportWarnOpen}
    title="DSH 正在运行"
    message="运行中导出可能备份到写到一半的会话文件，建议先停止 DSH 再导出。仍要继续导出吗？"
    confirmLabel="继续导出"
    onconfirm={() => {
      exportWarnOpen = false;
      void runExport();
    }}
    oncancel={() => (exportWarnOpen = false)}
  />

  <ConfirmDialog
    open={restoreSummary !== null}
    title="确认还原这份备份？"
    danger
    confirmLabel="停止 DSH 并还原"
    message={restoreMessage}
    onconfirm={() => void confirmRestore()}
    oncancel={() => (restoreSummary = null)}
  />

  <ConfirmDialog
    open={restoreReport !== null}
    title="还原完成"
    confirmLabel="重启千寻"
    message={restartMessage}
    onconfirm={() => void restartNow()}
    oncancel={dismissRestoreReport}
  />

  <section class="qx-card space-y-1 p-4 text-xs text-muted">
    <h2 class="qx-h">关于</h2>
    {#if meta}
      <p>千寻 v{meta.version} · {meta.identifier}</p>
      <p>Tauri 2 · Svelte 5</p>
    {:else}
      <p>版本信息不可用</p>
    {/if}
  </section>
</section>
