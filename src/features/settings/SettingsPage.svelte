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
    SyncItemResult,
    SyncStatus,
    ThemePreference,
  } from '../../lib/ipc/contract';
  import { nav } from '../../stores/nav.svelte';
  import { settings } from '../../stores/settings.svelte';
  import { theme } from '../../stores/theme.svelte';
  import ConfirmDialog from '../../components/ConfirmDialog.svelte';
  import Switch from '../../components/Switch.svelte';
  import SettingRow from './SettingRow.svelte';

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

  /** 主题缩略图配色（固定色值，不随当前主题切换）。 */
  const PREVIEW = {
    light: { win: '#f5f5f8', side: '#e9e9ef', ink: '#c9c9d4' },
    dark: { win: '#17181d', side: '#101116', ink: '#3c3d47' },
  } as const;

  // ---- 页面骨架：左侧分区导航 + 滚动联动 ----
  const SECTIONS: Array<{ id: string; label: string; icon: string[] }> = [
    {
      id: 'appearance',
      label: '外观',
      icon: [
        'M12 3a9 9 0 019 9 4 4 0 01-4 4h-2a2 2 0 00-1.6 3.2A2 2 0 0112.5 21 9 9 0 013 12a9 9 0 019-9z',
        'M7.5 11h.01',
        'M10.5 7.5h.01',
        'M14.5 7.5h.01',
      ],
    },
    {
      id: 'general',
      label: '通用',
      icon: ['M4 7h9', 'M17 7h3', 'M4 17h3', 'M11 17h9', 'M15 5v4', 'M7 15v4'],
    },
    {
      id: 'dsh',
      label: 'DSH 环境',
      icon: ['M12 3l8 4.5v9L12 21l-8-4.5v-9L12 3z', 'M12 12l8-4.5', 'M12 12v9', 'M12 12L4 7.5'],
    },
    {
      id: 'web',
      label: '联网',
      icon: [
        'M12 3a9 9 0 100 18 9 9 0 000-18z',
        'M3 12h18',
        'M12 3c2.5 2.6 3.8 5.7 3.8 9S14.5 18.4 12 21c-2.5-2.6-3.8-5.7-3.8-9S9.5 5.6 12 3z',
      ],
    },
    {
      id: 'data',
      label: '数据',
      icon: ['M4 8v11a1 1 0 001 1h14a1 1 0 001-1V8', 'M3 4h18v4H3z', 'M10 12h4'],
    },
    {
      id: 'about',
      label: '关于',
      icon: ['M12 3a9 9 0 100 18 9 9 0 000-18z', 'M12 8h.01', 'M12 11.5V16'],
    },
  ];

  const sectionEls: Record<string, HTMLElement | undefined> = {};
  let pageEl: HTMLElement | undefined = $state();
  let activeSection = $state('appearance');

  /** 分区导航：锚点滚动 + 即时高亮（尊重减少动效偏好）。 */
  function gotoSection(id: string): void {
    const el = sectionEls[id];
    if (!el) return;
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    activeSection = id;
    // 页尾分区受滚动高度限制可能到不了判定线，锚点滚动期间锁定高亮，
    // 避免联动把用户刚点的分区翻掉；用户再次主动滚动即恢复联动。
    spyPinned = true;
    el.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'start' });
  }

  // 滚动联动：挂到实际滚动容器（App 的 overflow-y-auto 包层），
  // 分区顶部越过视口判定线即切换高亮。页面 keep-alive 隐藏时不触发。
  let spyPinned = false;
  $effect(() => {
    const page = pageEl;
    if (!page) return;
    const scroller = page.closest('.overflow-y-auto') as HTMLElement | null;
    if (!scroller) return;
    const SPY_LINE = 150;
    const onScroll = (): void => {
      if (spyPinned) return;
      // 贴底特判：页尾分区受滚动高度限制永远越不过判定线，直接激活最后一节。
      if (scroller.scrollTop + scroller.clientHeight >= scroller.scrollHeight - 8) {
        activeSection = SECTIONS[SECTIONS.length - 1]?.id ?? 'about';
        return;
      }
      let current = SECTIONS[0]?.id ?? 'appearance';
      for (const { id } of SECTIONS) {
        const el = sectionEls[id];
        if (el && el.getBoundingClientRect().top <= SPY_LINE) current = id;
      }
      activeSection = current;
    };
    const wake = (): void => {
      spyPinned = false;
      onScroll();
    };
    onScroll();
    scroller.addEventListener('scroll', onScroll, { passive: true });
    scroller.addEventListener('wheel', wake, { passive: true });
    scroller.addEventListener('pointerdown', wake, { passive: true });
    scroller.addEventListener('keydown', wake);
    return () => {
      scroller.removeEventListener('scroll', onScroll);
      scroller.removeEventListener('wheel', wake);
      scroller.removeEventListener('pointerdown', wake);
      scroller.removeEventListener('keydown', wake);
    };
  });

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

  /** 与已保存值不一致时才允许「应用」，避免无意义的重复注册。 */
  const hotkeyDirty = $derived(hotkeyDraft.trim() !== (settings.current?.hotkeys.screenshot ?? ''));

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
  /** 导出含明文 API Key 的知情确认（docs/18 A2）。 */
  let credentialWarnOpen = $state(false);

  const BACKUP_FILTER = { name: '千寻备份', extensions: ['zip'] };

  const restoreMessage = $derived(
    restoreSummary
      ? `${restoreSummary.createdAt} · 千寻 v${restoreSummary.appVersion}` +
          `${restoreSummary.dshVersion ? ` · DSH ${restoreSummary.dshVersion}` : ''}` +
          ` · ${restoreSummary.fileCount} 个文件。` +
          '将覆盖全部设置与 DSH 数据（含 API Key）；还原前自动快照，DSH 会先停止。'
      : '',
  );

  const restartMessage = $derived(
    restoreReport
      ? `已还原 ${restoreReport.restoredFiles} 个文件` +
          (restoreReport.preRestoreBackup ? `，快照已存 ${restoreReport.preRestoreBackup}` : '') +
          (restoreReport.pluginsMissing > 0
            ? `，${restoreReport.pluginsMissing} 个插件待补装`
            : '') +
          '。重启后生效。'
      : '',
  );

  // ---- 清单补装（08 设计 §5.2）：还原报告里 N > 0 时给一键补装 --------
  let pinningBack = $state(false);
  let pinBackMessage = $state('');
  /** 最近一次还原的清单缺失数（>0 时在数据区显示补装入口；随还原更新）。 */
  let restoreMissing = $state(0);

  async function pinBack(): Promise<void> {
    pinningBack = true;
    pinBackMessage = '';
    try {
      const results = await call<SyncItemResult[]>('market_sync_pinned');
      const failed = results.filter((item) => !item.ok);
      if (failed.length === 0) {
        restoreMissing = 0;
        pinBackMessage = `补装完成：${results.length} 项全部就绪`;
      } else {
        pinBackMessage = `补装完成：${results.length - failed.length} 成功 · ${failed.length} 失败，可重试`;
      }
    } catch (error) {
      pinBackMessage = error instanceof Error ? error.message : String(error);
    } finally {
      pinningBack = false;
    }
  }

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
    // 凭据确认（docs/18 A2）：备份包必然含 .credentials.yaml 明文 API Key
    // （backup::excluded 只排 node_modules/logs/*.tmp），原先只有一行 warn
    // 文案、无 danger 确认。凭据外泄是不可逆的，先让用户明确知情。
    credentialWarnOpen = true;
  }

  /** 凭据确认通过后再判 DSH 是否在跑（两层提醒各自独立）。 */
  async function afterCredentialAck(): Promise<void> {
    // 会话是追加写：运行中导出可能备份到半截文件，再提醒一次。
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
      restoreMissing = restoreReport.pluginsMissing;
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

{#snippet miniWin(mode: 'light' | 'dark')}
  {@const c = PREVIEW[mode]}
  <span
    class="flex h-full min-w-0 flex-1 overflow-hidden rounded-md border"
    style="border-color:{c.ink};background:{c.win}"
  >
    <span class="block w-[38%] shrink-0" style="background:{c.side}"></span>
    <span class="flex min-w-0 flex-1 flex-col justify-center gap-1 py-1.5 pl-1.5 pr-2">
      <span class="qx-grad block h-1 w-full rounded-full"></span>
      <span class="block h-1 w-3/5 rounded-full" style="background:{c.ink}"></span>
      <span class="block h-1 w-2/5 rounded-full" style="background:{c.ink}"></span>
    </span>
  </span>
{/snippet}

<section class="mx-auto max-w-4xl pb-4" bind:this={pageEl}>
  <h1 class="qx-page-title">设置</h1>

  {#if settings.loadError}
    <div class="mt-4 rounded-xl border border-danger/40 bg-danger/10 p-4 text-sm">
      <p class="font-medium text-danger">设置加载失败</p>
      <p class="mt-1 text-muted">{settings.loadError}</p>
      <button class="qx-btn qx-btn-outline qx-btn-md mt-2" onclick={() => void settings.load()}>
        重试
      </button>
    </div>
  {/if}

  {#if saveError}
    <div
      class="mt-4 rounded-xl border border-danger/40 bg-danger/10 px-4 py-3 text-sm text-danger"
      role="alert"
    >
      保存失败：{saveError}
    </div>
  {/if}

  {#if !settings.current && !settings.loadError}
    <div class="flex items-center gap-2.5 py-20 text-sm text-muted">
      <svg class="size-4 animate-spin" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="2.5" class="opacity-20" />
        <path
          d="M21 12a9 9 0 00-9-9"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
        />
      </svg>
      正在加载设置…
    </div>
  {/if}

  <div class="mt-5 flex items-start gap-8">
    <!-- 分区导航：宽窗口展示，窄窗口自动退化为单列滚动。 -->
    <nav class="sticky top-6 hidden w-40 shrink-0 lg:block" aria-label="设置分区">
      <div class="flex flex-col gap-0.5">
        {#each SECTIONS as s (s.id)}
          <button
            class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-sm transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent/50 {activeSection ===
            s.id
              ? 'bg-accent-soft font-medium text-accent'
              : 'text-muted hover:bg-accent-soft/40 hover:text-fg'}"
            onclick={() => gotoSection(s.id)}
          >
            <svg
              viewBox="0 0 24 24"
              class="size-4 shrink-0"
              fill="none"
              stroke="currentColor"
              stroke-width="1.7"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              {#each s.icon as d, iconIndex (iconIndex)}
                <path {d} />
              {/each}
            </svg>
            {s.label}
          </button>
        {/each}
      </div>
    </nav>

    <div class="min-w-0 flex-1 space-y-5">
      {#if settings.current}
        <!-- 外观 -->
        <section
          id="appearance"
          bind:this={sectionEls.appearance}
          class="qx-card scroll-mt-6 px-5 pt-4 pb-4"
        >
          <h2 class="qx-h pb-2.5">外观</h2>
          <div class="grid grid-cols-3 gap-2.5" role="group" aria-label="主题">
            {#each themeOptions as option (option.value)}
              {@const active = theme.preference === option.value}
              <button
                type="button"
                class="rounded-xl border p-2 text-left transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent/50 {active
                  ? 'border-accent/60 bg-accent-soft/30'
                  : 'border-line hover:border-accent/40'}"
                aria-pressed={active}
                aria-label="主题：{option.label}"
                onclick={() => onTheme(option.value)}
              >
                <span class="block h-12" aria-hidden="true">
                  {#if option.value === 'system'}
                    <span class="flex h-full gap-1">
                      {@render miniWin('light')}
                      {@render miniWin('dark')}
                    </span>
                  {:else}
                    <span class="flex h-full">
                      {@render miniWin(option.value)}
                    </span>
                  {/if}
                </span>
                <span class="mt-2 flex items-center justify-between gap-1">
                  <span class="text-xs {active ? 'font-medium text-accent' : 'text-muted'}">
                    {option.label}
                  </span>
                  <span
                    class="flex size-4 items-center justify-center {active
                      ? 'text-accent'
                      : 'text-transparent'}"
                  >
                    <svg
                      viewBox="0 0 24 24"
                      class="size-3.5"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="2.2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true"
                    >
                      <path d="M5 12.5l4.5 4.5L19 7.5" />
                    </svg>
                  </span>
                </span>
              </button>
            {/each}
          </div>
        </section>

        <!-- 通用 -->
        <section
          id="general"
          bind:this={sectionEls.general}
          class="qx-card scroll-mt-6 px-5 pt-4 pb-3.5"
        >
          <h2 class="qx-h pb-2">通用</h2>
          <div class="divide-y divide-line/60">
            <SettingRow label="关闭时隐藏到托盘">
              <Switch
                label="关闭时隐藏到托盘"
                checked={settings.current.window.closeToTray}
                onchange={(value) => void save({ window: { closeToTray: value } })}
              />
            </SettingRow>
            <SettingRow label="启动时最小化">
              <Switch
                label="启动时最小化"
                checked={settings.current.window.startMinimized}
                onchange={(value) => void save({ window: { startMinimized: value } })}
              />
            </SettingRow>
            <SettingRow label="截屏热键" hint="清空后应用即停用">
              <input
                class="qx-input w-40 text-center font-mono {hotkeyRecording
                  ? 'border-accent/60 motion-safe:animate-pulse'
                  : ''}"
                type="text"
                readonly
                placeholder="点击录制"
                aria-label="截屏热键"
                value={hotkeyRecording ? '按下组合键，Esc 取消' : hotkeyDraft}
                onclick={() => (hotkeyRecording = true)}
                onkeydown={onHotkeyKeydown}
              />
              <button
                class="qx-btn qx-btn-primary qx-btn-md"
                disabled={!hotkeyDirty || hotkeyRecording}
                onclick={() => void applyHotkey()}
              >
                应用
              </button>
            </SettingRow>
            {#if hotkeyError}
              <div class="py-3 text-sm text-danger">{hotkeyError}</div>
            {/if}
            <SettingRow label="远程访问">
              <button class="qx-btn qx-btn-outline qx-btn-md" onclick={() => nav.go('remote')}>
                打开远程页
              </button>
            </SettingRow>
          </div>
        </section>

        <!-- DSH 环境 -->
        <section id="dsh" bind:this={sectionEls.dsh} class="qx-card scroll-mt-6 px-5 pt-4 pb-3.5">
          <h2 class="qx-h pb-2">DSH 环境</h2>
          <div class="divide-y divide-line/60">
            <SettingRow label="随千寻启动">
              <Switch
                label="随千寻启动 DSH"
                checked={settings.current.dsh.autostart}
                onchange={(value) => void save({ dsh: { autostart: value } })}
              />
            </SettingRow>
            <SettingRow label="数据目录" hint="隔离 = 千寻专用，不影响系统 ~/.dsh">
              <select
                class="qx-select w-48"
                aria-label="DSH 数据目录"
                value={settings.current.dsh.home}
                onchange={(event) => {
                  const value = event.currentTarget.value as 'isolated' | 'system';
                  void save({ dsh: { home: value } });
                }}
              >
                <option value="isolated">隔离（推荐）</option>
                <option value="system">系统 ~/.dsh</option>
              </select>
            </SettingRow>
            <SettingRow label="服务端口">
              <span class="font-mono text-xs text-muted">随机分配</span>
            </SettingRow>
            <SettingRow label="锁定版本">
              <span
                class="max-w-56 truncate font-mono text-xs text-muted"
                title="由千寻版本锁定，不可修改"
              >
                {installSpec ?? '…'}
              </span>
            </SettingRow>
            <SettingRow label="Node 下载源">
              <select
                class="qx-select w-44"
                aria-label="Node 下载源"
                value={settings.current.mirrors.nodeBinary}
                onchange={(event) => {
                  const value = event.currentTarget.value as 'auto' | 'official' | 'npmmirror';
                  void save({ mirrors: { nodeBinary: value } });
                }}
              >
                <option value="auto">自动（镜像优先）</option>
                <option value="official">官方源</option>
                <option value="npmmirror">npmmirror</option>
              </select>
            </SettingRow>
            <SettingRow
              label="npm registry"
              error={registryValid ? '' : '仅限 npmmirror、official 或 http(s):// 地址'}
            >
              <input
                class="qx-input w-56 font-mono text-xs invalid:border-danger"
                type="text"
                list="registry-presets"
                aria-label="npm registry"
                bind:value={registryInput}
                onblur={() => {
                  if (registryValid) void save({ mirrors: { npmRegistry: registryInput.trim() } });
                }}
              />
              <datalist id="registry-presets">
                <option value="npmmirror">npmmirror（淘宝源）</option>
                <option value="official">npm 官方</option>
              </datalist>
            </SettingRow>
          </div>
        </section>

        <!-- 联网搜索 -->
        <section
          id="web"
          bind:this={sectionEls.web}
          class="qx-card scroll-mt-6 px-5 pt-4 pb-3.5"
          data-testid="settings-web"
        >
          <h2 class="qx-h pb-2">联网搜索</h2>
          <div class="divide-y divide-line/60">
            <SettingRow
              label="默认引擎"
              hint={`与 DSH web_search 共用清单 · 已装 ${settings.current.web.engines.length} 个 · 增删请编辑 settings.json`}
            >
              <select
                class="qx-select w-44"
                aria-label="默认搜索引擎"
                data-testid="web-default-engine"
                value={settings.current.web.defaultEngine}
                onchange={(event) => {
                  const value = event.currentTarget.value;
                  void save({ web: { defaultEngine: value } });
                }}
              >
                {#each settings.current.web.engines as engine (engine.id)}
                  <option value={engine.id}>{engine.id}</option>
                {/each}
              </select>
            </SettingRow>
            <SettingRow label="Moli 路径" fill>
              <input
                class="qx-input min-w-0 flex-1 font-mono"
                type="text"
                placeholder="留空 = 受管安装（tools/moli/）"
                aria-label="Moli 自备路径"
                data-testid="web-moli-path"
                value={settings.current.tools.moli.binaryPath ?? ''}
                onchange={(event) => {
                  const value = event.currentTarget.value.trim();
                  void save({ tools: { moli: { binaryPath: value } } });
                }}
              />
            </SettingRow>
            <SettingRow label="抓取代理" hint="仅 Moli 抓取走此代理" fill>
              <input
                class="qx-input min-w-0 flex-1 font-mono"
                type="text"
                placeholder="socks5://127.0.0.1:1080 · 留空直连"
                title="TUN/fake-ip 网络下直连会被内网安全守卫拦截，填本机代理端口即可"
                aria-label="抓取代理"
                data-testid="web-proxy"
                value={settings.current.web.proxy ?? ''}
                onchange={(event) => {
                  const value = event.currentTarget.value.trim();
                  void save({ web: { proxy: value } });
                }}
              />
            </SettingRow>
          </div>
        </section>
      {/if}

      <!-- 数据：笔记同步 + 备份还原 -->
      <section id="data" bind:this={sectionEls.data} class="qx-card scroll-mt-6 px-5 pt-4 pb-3.5">
        <h2 class="qx-h pb-2">数据</h2>
        <div class="divide-y divide-line/60">
          {#if syncStatus}
            {#if !syncStatus.gitAvailable}
              <SettingRow label="笔记同步" error="未检测到 git，安装后重试" />
            {:else if !syncStatus.initialized}
              <SettingRow label="笔记同步" hint="笔记库尚未纳入 git">
                <button
                  class="qx-btn qx-btn-primary qx-btn-md"
                  disabled={syncBusy || !vaultReady}
                  title={vaultReady ? '' : '先在「笔记」页设置仓库目录'}
                  onclick={() => void syncAction('sync_init')}
                >
                  初始化
                </button>
              </SettingRow>
            {:else}
              <SettingRow
                label="笔记同步"
                hint={`未提交 ${syncStatus.dirty} 处 · ${
                  syncStatus.hasRemote
                    ? `领先 ${syncStatus.ahead ?? '?'} · 落后 ${syncStatus.behind ?? '?'}`
                    : '无远端'
                }`}
              >
                <button
                  class="qx-btn qx-btn-primary qx-btn-md"
                  disabled={syncBusy || !syncStatus.hasRemote}
                  title={syncStatus.hasRemote ? '' : '先给仓库添加 remote'}
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
                <button
                  class="qx-icon-btn"
                  aria-label="刷新同步状态"
                  title="刷新"
                  onclick={() => void refreshSync()}
                >
                  <svg
                    viewBox="0 0 24 24"
                    class="size-4"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.7"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    <path d="M20 11a8 8 0 10-2.3 5.7M20 5v6h-6" />
                  </svg>
                </button>
              </SettingRow>
            {/if}
          {:else}
            <SettingRow label="笔记同步" hint="读取状态…">
              <button
                class="qx-icon-btn"
                aria-label="刷新同步状态"
                title="刷新"
                onclick={() => void refreshSync()}
              >
                <svg
                  viewBox="0 0 24 24"
                  class="size-4"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.7"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <path d="M20 11a8 8 0 10-2.3 5.7M20 5v6h-6" />
                </svg>
              </button>
            </SettingRow>
          {/if}
          {#if syncError}
            <div class="py-3 text-sm text-danger">{syncError}</div>
          {/if}
          {#if syncLog.length > 0}
            <div class="py-3">
              <pre
                class="max-h-32 overflow-y-auto rounded-lg bg-bg/70 p-2.5 font-mono text-xs leading-5 text-muted">{syncLog.join(
                  '\n',
                )}</pre>
            </div>
          {/if}

          <SettingRow
            label="备份与还原"
            hint="千寻设置 + DSH 用户数据 · zip 包"
            warn="备份含 API Key 明文，妥善保管"
          >
            <button
              class="qx-btn qx-btn-primary qx-btn-md"
              disabled={backupBusy}
              onclick={() => void exportBackup()}
            >
              导出…
            </button>
            <button
              class="qx-btn qx-btn-outline qx-btn-md"
              disabled={backupBusy}
              onclick={() => void pickRestore()}
            >
              还原…
            </button>
          </SettingRow>
          {#if exportResult}
            <div class="py-3 text-xs leading-5 break-all text-muted">
              已导出 {exportResult.path}（{formatSize(exportResult.sizeBytes)} · {exportResult.fileCount}
              个文件）
            </div>
          {/if}
          {#if backupError}
            <div class="py-3 text-sm text-danger">{backupError}</div>
          {/if}
          {#if restoreMissing > 0 && restoreReport === null}
            <div
              class="flex items-center justify-between gap-3 rounded-lg border border-warning/40 bg-warning/10 px-3 py-2.5 my-2"
            >
              <p class="text-xs text-warning">上次还原有 {restoreMissing} 个插件未落盘</p>
              <button
                class="qx-btn qx-btn-outline qx-btn-sm shrink-0"
                disabled={pinningBack}
                data-testid="backup-pin-back"
                onclick={() => void pinBack()}
              >
                {pinningBack ? '补装中…' : `补装（${restoreMissing}）`}
              </button>
            </div>
          {/if}
          {#if pinBackMessage}
            <p class="py-2 text-xs text-muted" role="status">{pinBackMessage}</p>
          {/if}
        </div>
      </section>

      <!-- 关于 -->
      <section
        id="about"
        bind:this={sectionEls.about}
        class="qx-card flex scroll-mt-6 items-center gap-4 px-5 py-4"
      >
        <div
          class="qx-grad flex size-11 shrink-0 items-center justify-center rounded-xl text-white shadow-lg shadow-accent/25"
          aria-hidden="true"
        >
          <svg
            viewBox="0 0 24 24"
            class="size-6"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 3a9 9 0 100 18 9 9 0 000-18z" />
            <path d="M15.5 8.5l-2.2 4.8-4.8 2.2 2.2-4.8z" />
          </svg>
        </div>
        {#if meta}
          <div class="min-w-0">
            <p class="text-sm font-medium text-fg">
              千寻 <span class="ml-1 font-mono text-xs text-muted">v{meta.version}</span>
            </p>
            <p class="mt-0.5 text-xs text-muted">{meta.identifier} · Tauri 2 · Svelte 5</p>
          </div>
        {:else}
          <p class="text-xs text-muted">版本信息不可用</p>
        {/if}
      </section>
    </div>
  </div>
</section>

<ConfirmDialog
  open={credentialWarnOpen}
  danger
  title="备份包含明文 API Key"
  confirmLabel="我已了解，继续导出"
  cancelLabel="取消"
  message="导出包会原样包含 DSH 的 .credentials.yaml（LLM / 供应商 API Key 明文）与全部设置。请只保存到本地可信位置，不要上传网盘或 IM。凭据一旦外泄无法从备份中撤回。"
  onconfirm={() => {
    credentialWarnOpen = false;
    void afterCredentialAck();
  }}
  oncancel={() => (credentialWarnOpen = false)}
/>

<ConfirmDialog
  open={exportWarnOpen}
  title="DSH 正在运行"
  message="运行中导出可能包含写了一半的会话文件。仍要继续？"
  confirmLabel="继续导出"
  onconfirm={() => {
    exportWarnOpen = false;
    void runExport();
  }}
  oncancel={() => (exportWarnOpen = false)}
/>

<ConfirmDialog
  open={restoreSummary !== null}
  title="确认还原？"
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
  cancelLabel="暂不"
  message={restartMessage}
  onconfirm={() => void restartNow()}
  oncancel={dismissRestoreReport}
/>
