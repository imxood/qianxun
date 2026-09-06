<script lang="ts">
  /**
   * 终端页（M4）：多标签宿主。标签条 + 全部 pane 常驻（CSS 隐藏保活）。
   * 运行中的标签关闭需确认（自定义对话框——WebView2 原生 confirm 不可靠）；
   * 退出的标签保留尾输出并标灰；启动失败的标签显示原因并可重试。
   *
   * 标签交互：双击重命名（手动标题不再被 OSC 标题覆盖）、右键菜单
   * （重命名 / 固定 / 取消固定 / 清空 / 分离或合并 / 关闭）、左右拖拽重排
   * （pointer 手势，移动 5px 进入拖拽，原位点击不受影响）。
   *
   * 多窗口：主窗标签可「分离到独立窗口」（transfer 到新独立终端窗），
   * 独立窗口标签可「合并到主窗口」。会话归属由 Rust 元数据管理；
   * 挂载时经 terminal_sessions 恢复本窗口名下的存活会话（重挂载/接管
   * 皆不出 orphan），PIN 记录全局唯一持有（被存活会话持有的不重复恢复）。
   */
  import { onMount } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
  import { listen } from '@tauri-apps/api/event';
  import { call } from '../../lib/ipc';
  import { settings } from '../../stores/settings.svelte';
  import { contextMenu } from '../../lib/menu.svelte';
  import { WINDOW_LABEL } from '../../lib/windowEnv';
  import { folderName, shellTitle } from '../../lib/utils/shell';
  import ConfirmDialog from '../../components/ConfirmDialog.svelte';
  import Switch from '../../components/Switch.svelte';
  import TerminalPane, { type PaneApi } from './TerminalPane.svelte';
  import type {
    PinnedTerminal,
    TerminalSessionSnapshot,
    TerminalTransferEvent,
  } from '../../lib/ipc/contract';

  let { standalone = false }: { standalone?: boolean } = $props();

  type Tab = {
    id: number;
    title: string;
    alive: boolean;
    error: string | null;
    /** 实际 shell（spawn 返回；PIN 恢复的 spawn 入参）。 */
    shell: string | null;
    /** 最近一次 OSC 7 上报的 cwd。 */
    cwd: string | null;
    /** 固定记录 id；null = 未固定。 */
    pinned: number | null;
    /** 手动重命名后，OSC 标题不再覆盖。 */
    manualTitle: boolean;
    /** 恢复自 PIN 记录：记录 id（同时标记需要写入历史）。 */
    restoreFrom: number | null;
    /** 恢复时写入 xterm 的历史内容（restoreFrom 的记录正文）。 */
    restoreHistory: string;
  };

  let tabs = $state<Tab[]>([]);
  let activeId = $state<number | null>(null);
  let closeTarget: Tab | null = $state(null);
  let renamingId = $state<number | null>(null);
  let renameDraft = $state('');
  /** 标签条右侧的终端设置弹层。 */
  let settingsOpen = $state(false);

  const paneApis = new SvelteMap<number, PaneApi>();

  /** 激活标签后把焦点交给终端（下一帧：等 DOM 切可见）。 */
  function focusPane(id: number): void {
    requestAnimationFrame(() => paneApis.get(id)?.focus());
  }

  const prefs = $derived(
    settings.current?.terminal ?? {
      shell: 'auto',
      fontSize: 13,
      scrollback: 5000,
      cursorStyle: 'block' as 'block' | 'bar' | 'underline',
      cursorBlink: true,
    },
  );

  // 焦点统一接管：activeId 的任何变化（点击切换 / 新建 / PIN 恢复链的
  // 最后一条 / 转移接管 / 关闭邻居后顺延）都把焦点交给当前终端。
  // 终端页不可见时（用户在看别的页）focus 无效但无害。
  $effect(() => {
    if (activeId !== null) focusPane(activeId);
  });

  onMount(() => {
    const disposers: Array<() => void> = [];
    // 先接管本窗口名下的存活会话（重挂载/独立窗口关闭后的恢复），
    // 再恢复无主的 PIN 记录（Rust 侧已把被持有的 PIN 过滤掉）。
    void recoverSessions()
      .catch(() => {})
      .then(() => restorePinned())
      .finally(() => {
        if (tabs.length === 0) void newTab();
      });
    // 跨窗口转移：目标是自己才接管（从别处分离/合并进来的会话）。
    void listen<TerminalTransferEvent>('terminal://transferred', (event) => {
      const payload = event.payload;
      if (payload.windowLabel !== WINDOW_LABEL) return;
      if (tabs.some((tab) => tab.id === payload.id)) return;
      adoptTransferred(payload);
    }).then((unlisten) => disposers.push(unlisten));
    return () => {
      for (const dispose of disposers) dispose();
    };
  });

  /** 把转移事件落成本地标签（xterm 历史由 pane 挂载时的 replay 自动补齐）。 */
  function adoptTransferred(payload: TerminalTransferEvent): void {
    tabs = [
      ...tabs,
      {
        id: payload.id,
        title: payload.title || shellTitle(payload.shell),
        alive: true,
        error: null,
        shell: payload.shell,
        cwd: payload.cwd,
        pinned: payload.pinId,
        manualTitle: true,
        restoreFrom: null,
        restoreHistory: '',
      },
    ];
    activeId = payload.id;
  }

  /** 重挂载恢复：Rust 元数据里归属本窗口的存活会话重建标签。 */
  async function recoverSessions(): Promise<void> {
    const sessions = await call<TerminalSessionSnapshot[]>('terminal_sessions', {});
    for (const session of sessions) {
      if (tabs.some((tab) => tab.id === session.id)) continue;
      tabs = [
        ...tabs,
        {
          id: session.id,
          title: session.title ?? shellTitle(session.shell),
          alive: true,
          error: null,
          shell: session.shell,
          cwd: session.cwd,
          pinned: session.pinId,
          manualTitle: session.title !== null,
          restoreFrom: null,
          restoreHistory: '',
        },
      ];
      activeId = session.id;
    }
  }

  async function restorePinned(): Promise<void> {
    let pinned: PinnedTerminal[];
    try {
      pinned = await call<PinnedTerminal[]>('terminal_pinned_list');
    } catch {
      return; // 恢复失败不阻塞新终端。
    }
    for (const record of pinned) {
      try {
        const info = await call<{ id: number; shell: string }>('terminal_spawn', {
          shell: record.shell,
          cwd: record.cwd,
          cols: 80,
          rows: 24,
        });
        const history = await call<string>('terminal_pinned_replay', { pinId: record.pinId });
        tabs = [
          ...tabs,
          {
            id: info.id,
            title: record.title || shellTitle(info.shell),
            alive: true,
            error: null,
            shell: info.shell,
            cwd: record.cwd,
            pinned: record.pinId,
            manualTitle: record.title !== '',
            restoreFrom: record.pinId,
            restoreHistory: history,
          },
        ];
        activeId = info.id;
        // 新会话续上 PIN 记录：下次退出时刷新回放（不覆盖恢复用的历史）。
        void call('terminal_pin_resume', { id: info.id, pinId: record.pinId }).catch(() => {});
      } catch {
        // 单条恢复失败（如 cwd 已不存在）：跳过，记录保留下次再试。
      }
    }
  }

  async function newTab(cwd: string | null = null): Promise<void> {
    try {
      const info = await call<{ id: number; shell: string }>('terminal_spawn', {
        shell: prefs.shell,
        cwd,
        cols: 80,
        rows: 24,
      });
      tabs = [
        ...tabs,
        {
          id: info.id,
          // 带 cwd 新建（继承激活终端目录）时标题即目录名；否则先显示
          // shell 名，首个提示符的 OSC 7 上报后再切到目录名。
          title: cwd ? folderName(cwd) : shellTitle(info.shell),
          alive: true,
          error: null,
          shell: info.shell,
          cwd,
          pinned: null,
          manualTitle: false,
          restoreFrom: null,
          restoreHistory: '',
        },
      ];
      activeId = info.id;
    } catch (error) {
      // 失败原因上屏（错误窗格），不再只进 console。
      const message = error instanceof Error ? error.message : String(error);
      tabs = [
        ...tabs,
        {
          id: -Date.now(),
          title: '启动失败',
          alive: false,
          error: message,
          shell: null,
          cwd: null,
          pinned: null,
          manualTitle: false,
          restoreFrom: null,
          restoreHistory: '',
        },
      ];
      activeId = tabs[tabs.length - 1]?.id ?? null;
    }
  }

  function startRename(tab: Tab): void {
    renamingId = tab.id;
    renameDraft = tab.title;
  }

  function commitRename(): void {
    const tab = tabs.find((item) => item.id === renamingId);
    if (tab && renameDraft.trim()) {
      tab.title = renameDraft.trim();
      tab.manualTitle = true;
    }
    renamingId = null;
  }

  function tabMenu(event: MouseEvent, tab: Tab): void {
    const items: Array<{ label: string; danger?: boolean; onclick?: () => void }> = [
      { label: '重命名', onclick: () => startRename(tab) },
    ];
    if (tab.alive) {
      items.push(
        tab.pinned === null
          ? { label: '固定（PIN）', onclick: () => void pinTab(tab) }
          : { label: '取消固定', onclick: () => void unpinTab(tab) },
      );
      items.push({ label: '清空', onclick: () => clearTab(tab) });
      // 跨窗口：主窗 = 分离此标签到独立窗口；独立窗 = 合并回主窗。
      if (!standalone) {
        items.push({ label: '分离到独立窗口', onclick: () => void detachTab(tab) });
      } else {
        items.push({ label: '合并到主窗口', onclick: () => void mergeTab(tab) });
      }
    }
    items.push({ label: '关闭', danger: true, onclick: () => requestClose(tab) });
    contextMenu.show(event, items);
  }

  /** 分离单个标签：新开独立终端窗口并把会话转移过去（进程不重启）。 */
  async function detachTab(tab: Tab): Promise<void> {
    if (!tab.alive) return;
    const label = await call<string>('window_spawn_view', { view: 'terminal' });
    await call('terminal_transfer', {
      id: tab.id,
      target: label,
      title: tab.title,
      shell: tab.shell ?? prefs.shell,
      cwd: tab.cwd,
      pinId: tab.pinned,
    });
    remove(tab.id); // 不 kill：会话已归新窗口。
  }

  /** 合并单个标签回主窗；独立窗口最后一个标签合并后自动关窗回家。 */
  async function mergeTab(tab: Tab): Promise<void> {
    if (!tab.alive) return;
    await call('terminal_transfer', {
      id: tab.id,
      target: 'main',
      title: tab.title,
      shell: tab.shell ?? prefs.shell,
      cwd: tab.cwd,
      pinId: tab.pinned,
    });
    remove(tab.id);
    if (standalone && tabs.length === 0) {
      await call('window_reveal_main');
      void call('window_force_close').catch(() => {});
    }
  }

  async function pinTab(tab: Tab): Promise<void> {
    if (!tab.alive) return;
    try {
      const { pinId } = await call<{ pinId: number }>('terminal_pin', {
        id: tab.id,
        title: tab.title,
        shell: tab.shell ?? prefs.shell,
        cwd: tab.cwd,
      });
      tab.pinned = pinId;
    } catch {
      // 固定失败（会话已退出等）：静默，标签保持未固定。
    }
  }

  async function unpinTab(tab: Tab): Promise<void> {
    try {
      await call('terminal_unpin', { id: tab.id });
    } finally {
      tab.pinned = null;
    }
  }

  function clearTab(tab: Tab): void {
    paneApis.get(tab.id)?.clear();
  }

  function requestClose(tab: Tab): void {
    if (tab.alive) {
      closeTarget = tab;
      return;
    }
    // 已退出的标签：PIN 记录已在会话退出时刷新，删 UI 即可（记录保留）。
    if (tab.pinned !== null) void unpinTabKeepRecord(tab);
    void removeTab(tab);
  }

  async function unpinTabKeepRecord(tab: Tab): Promise<void> {
    // 已退出的固定标签：记录是恢复的凭据，只解除前端关联。
    tab.pinned = null;
  }

  function confirmClose(): void {
    const tab = closeTarget;
    closeTarget = null;
    if (tab) {
      // 活标签被用户关闭 = 不再想要：PIN 记录一并删除。
      if (tab.pinned !== null) void call('terminal_unpin', { id: tab.id }).catch(() => {});
      void removeTab(tab);
    }
  }

  async function removeTab(tab: Tab): Promise<void> {
    try {
      await call('terminal_kill', { id: tab.id });
    } finally {
      remove(tab.id);
    }
  }

  function retry(tab: Tab): void {
    remove(tab.id);
    void newTab();
  }

  function remove(id: number): void {
    paneApis.delete(id);
    tabEls.delete(id);
    const index = tabs.findIndex((tab) => tab.id === id);
    if (index < 0) return;
    tabs = tabs.filter((tab) => tab.id !== id);
    if (activeId === id) {
      activeId = tabs[Math.min(index, tabs.length - 1)]?.id ?? null;
    }
  }

  function onPaneExit(id: number): void {
    // 进程退出：标记死亡，保留 pane 显示尾输出，标签条变灰。
    const tab = tabs.find((item) => item.id === id);
    if (tab) tab.alive = false;
  }

  function onPaneTitle(id: number, title: string): void {
    const tab = tabs.find((item) => item.id === id);
    if (!tab || !title.trim() || tab.manualTitle) return;
    // cwd 已知时标题权威是「目录名」（OSC 7 驱动）；nushell 等会把 OSC
    // 标题设成完整路径，不能让它盖掉。cwd 未知时才拿 OSC 标题垫底。
    if (tab.cwd === null) tab.title = title;
  }

  function onPaneCwd(id: number, cwd: string): void {
    const tab = tabs.find((item) => item.id === id);
    if (!tab) return;
    tab.cwd = cwd;
    // 未手动重命名的标签跟随当前目录名（cd 一次标题就同步一次）。
    if (!tab.manualTitle) tab.title = folderName(cwd);
  }

  function onPaneBind(id: number, api: PaneApi): void {
    paneApis.set(id, api);
  }

  /** 行内重命名输入框挂载即聚焦。 */
  function focusNow(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  // ---- 标签拖拽重排（pointer 手势，与重命名/右键/关闭兼容）----
  // 按住移动 5px 进入拖拽，数组原位重排（keyed each 复用 DOM，pane 不重建）；
  // 未移动的原位松手 = 普通点击（激活标签），拖拽过的松手不触发点击。

  let dragId: number | null = $state(null);
  let dragStartX = 0;
  let dragStartY = 0;
  let dragMoved = false;
  let suppressClick = false;
  /**
   * 拖拽影像。位置不走 Svelte 状态：进入拖拽时渲染一次，之后 pointermove
   * 直接写 ghost 元素的 left（每 move 一次 style 写入，零状态开销）；
   * 垂直位置固定在标签条中心，只有水平跟随——上下晃动不难受。
   */
  let ghost: { title: string; pinned: boolean; y: number } | null = $state(null);
  let ghostEl: HTMLDivElement | null = null;
  /** 拖拽期间各标签的中点缓存（重排后刷新），move 里不再强制 layout。 */
  let cachedMids: Array<{ id: number; mid: number }> = [];
  const tabEls = new SvelteMap<number, HTMLElement>();

  function tabRef(node: HTMLElement, id: number): { destroy(): void } {
    tabEls.set(id, node);
    return {
      destroy() {
        tabEls.delete(id);
      },
    };
  }

  /** 结束拖拽：影像消散、状态复位（重排结果保留）。 */
  function endDrag(): void {
    dragId = null;
    dragMoved = false;
    ghost = null;
    ghostEl = null;
    cachedMids = [];
  }

  function tabPointerDown(event: PointerEvent, tab: Tab): void {
    if (event.button !== 0) return;
    // × 关闭钮自带点击语义，不作为拖拽起点。
    if ((event.target as HTMLElement).closest('[data-no-drag]')) return;
    dragId = tab.id;
    dragMoved = false;
    dragStartX = event.clientX;
    dragStartY = event.clientY;
    ghost = null;
    (event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId);
  }

  /** 缓存各标签中点（进入拖拽时 + 每次重排后调用一次）。 */
  function refreshMids(): void {
    cachedMids = tabs.map((tab) => {
      const rect = tabEls.get(tab.id)?.getBoundingClientRect();
      return { id: tab.id, mid: rect ? rect.left + rect.width / 2 : 0 };
    });
  }

  function tabPointerMove(event: PointerEvent): void {
    if (dragId === null) return;
    if (!dragMoved) {
      const dx = event.clientX - dragStartX;
      const dy = event.clientY - dragStartY;
      if (Math.hypot(dx, dy) < 5) return;
      dragMoved = true;
      // 影像只渲染一次；垂直锚定标签条中心，此后仅水平跟随鼠标。
      const dragged = tabs.find((tab) => tab.id === dragId);
      const barY = tabEls.get(dragId)?.getBoundingClientRect();
      ghost = {
        title: dragged?.title ?? '',
        pinned: dragged?.pinned !== null,
        y: barY ? barY.top + barY.height / 2 : event.clientY,
      };
      refreshMids();
    }
    // 直接写 style：绕过状态更新，move 高频路径零渲染开销。
    if (ghostEl) ghostEl.style.left = `${event.clientX + 10}px`;
    const from = tabs.findIndex((tab) => tab.id === dragId);
    if (from < 0) return;
    const target = insertionIndex(event.clientX);
    // 目标位与当前位相同（含 ±1 边界等价）则不动。
    if (target === from || target === from + 1) return;
    const next = [...tabs];
    const [moved] = next.splice(from, 1);
    next.splice(from < target ? target - 1 : target, 0, moved!);
    tabs = next;
    refreshMids();
  }

  /** 指针 x 落点对应的插入下标（缓存的标签中点为界；越界 = 尾部）。 */
  function insertionIndex(x: number): number {
    const index = cachedMids.findIndex((entry) => x < entry.mid);
    return index === -1 ? tabs.length : index;
  }

  function tabPointerUp(event: PointerEvent, tab: Tab): void {
    if (dragId !== tab.id) return;
    suppressClick = dragMoved;
    const button = event.currentTarget as HTMLButtonElement;
    if (button.hasPointerCapture(event.pointerId)) {
      button.releasePointerCapture(event.pointerId);
    }
    endDrag();
  }

  /** 拖拽中被系统打断（窗口失焦/右键等）：复位，不留悬挂影像。 */
  function tabPointerCancel(): void {
    suppressClick = false;
    endDrag();
  }

  /** Esc 取消拖拽：影像消散，已发生的重排保留。 */
  function cancelDragOnEscape(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || dragId === null) return;
    suppressClick = true;
    endDrag();
  }

  function tabClick(tab: Tab): void {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    activeId = tab.id;
  }

  // ---- 标签条设置弹层：改动即时持久化，TerminalPane 热应用 ----

  function saveTerminalPrefs(patch: {
    fontSize?: number;
    cursorStyle?: 'block' | 'bar' | 'underline';
    cursorBlink?: boolean;
    scrollback?: number;
  }): void {
    void settings.update({ terminal: patch }).catch(() => {});
  }

  const adjustFontSize = (delta: number): void => {
    saveTerminalPrefs({ fontSize: Math.min(24, Math.max(8, prefs.fontSize + delta)) });
  };

  const scrollbackOptions = [1000, 5000, 10000, 50000];
</script>

<svelte:window onkeydown={cancelDragOnEscape} />

<section class="flex h-full flex-col overflow-hidden">
  <div
    class="relative flex select-none items-center gap-1 border-b border-line bg-surface px-2 py-1.5"
    role="tablist"
  >
    {#each tabs as tab (tab.id)}
      {#if renamingId === tab.id}
        <!-- 双击进入行内重命名；Enter/失焦提交，Esc 取消。 -->
        <input
          class="w-36 rounded-md border border-accent bg-surface px-2 py-1 text-xs text-fg outline-none"
          bind:value={renameDraft}
          use:focusNow
          onkeydown={(event) => {
            if (event.key === 'Enter') commitRename();
            if (event.key === 'Escape') renamingId = null;
          }}
          onblur={commitRename}
        />
      {:else}
        <button
          use:tabRef={tab.id}
          class="group flex max-w-44 cursor-default items-center gap-1.5 rounded-md px-2.5 py-1 text-xs transition-colors {activeId ===
          tab.id
            ? 'bg-accent-soft font-medium text-fg'
            : 'text-muted hover:bg-accent-soft/60'} {dragId === tab.id
            ? 'relative z-0 opacity-30'
            : ''}"
          role="tab"
          aria-selected={activeId === tab.id}
          data-testid="terminal-tab"
          data-tab-id={tab.id}
          onclick={() => tabClick(tab)}
          ondblclick={() => startRename(tab)}
          oncontextmenu={(event) => tabMenu(event, tab)}
          onpointerdown={(event) => tabPointerDown(event, tab)}
          onpointermove={tabPointerMove}
          onpointerup={(event) => tabPointerUp(event, tab)}
          onpointercancel={tabPointerCancel}
          title="{tab.cwd ?? tab.title}{tab.pinned !== null
            ? '（已固定）'
            : ''}——双击重命名，右键更多，拖拽排序"
        >
          {#if tab.pinned !== null}
            <span class="shrink-0 text-[10px] text-accent" title="已固定">📌</span>
          {/if}
          <span class="truncate {tab.alive ? '' : 'opacity-50 line-through'}">{tab.title}</span>
          <span
            class="ml-0.5 rounded px-1 text-muted/70 opacity-0 transition-opacity hover:bg-black/10 group-hover:opacity-100"
            role="button"
            tabindex="-1"
            aria-label="关闭标签"
            data-no-drag
            onclick={(event) => {
              event.stopPropagation();
              requestClose(tab);
            }}>×</span
          >
        </button>
      {/if}
    {/each}
    <!-- + 紧邻标签列表（好点）；⚙ 独占行尾（ml-auto 撑开弹性空隙）。 -->
    <button
      class="rounded-md px-2 py-1 text-sm text-muted transition-colors hover:bg-accent-soft hover:text-fg"
      title="新建终端（继承当前终端的工作目录）"
      data-testid="terminal-new"
      onclick={() => void newTab(tabs.find((tab) => tab.id === activeId)?.cwd ?? null)}
    >
      +
    </button>
    <button
      class="ml-auto rounded-md px-1.5 py-1 text-sm text-muted transition-colors hover:bg-accent-soft hover:text-fg {settingsOpen
        ? 'bg-accent-soft text-fg'
        : ''}"
      title="终端设置"
      aria-label="终端设置"
      data-testid="terminal-settings"
      onclick={() => (settingsOpen = !settingsOpen)}
    >
      ⚙
    </button>
    {#if settingsOpen}
      <!-- 点击别处关闭（透明垫层）；面板悬浮在按钮下方。 -->
      <button
        class="fixed inset-0 z-40 cursor-default"
        aria-label="关闭终端设置"
        onclick={() => (settingsOpen = false)}
      ></button>
      <div
        class="absolute top-full z-50 mt-1 w-64 space-y-3 rounded-lg border border-line bg-card p-3 shadow-xl"
      >
        <div class="flex items-center justify-between text-xs">
          <span class="text-muted">字号</span>
          <span class="flex items-center gap-1.5">
            <button
              class="rounded border border-line px-1.5 hover:bg-accent-soft"
              aria-label="减小字号"
              onclick={() => adjustFontSize(-1)}>−</button
            >
            <span class="w-8 text-center font-mono">{prefs.fontSize}</span>
            <button
              class="rounded border border-line px-1.5 hover:bg-accent-soft"
              aria-label="增大字号"
              onclick={() => adjustFontSize(1)}>+</button
            >
          </span>
        </div>
        <div class="flex items-center justify-between text-xs">
          <span class="text-muted">光标样式</span>
          <select
            class="rounded border border-line bg-surface px-1.5 py-0.5"
            value={prefs.cursorStyle}
            onchange={(event) =>
              saveTerminalPrefs({
                cursorStyle: event.currentTarget.value as 'block' | 'bar' | 'underline',
              })}
          >
            <option value="block">块</option>
            <option value="bar">竖线</option>
            <option value="underline">下划线</option>
          </select>
        </div>
        <label class="flex items-center justify-between text-xs">
          <span class="text-muted">光标闪烁</span>
          <Switch
            label="光标闪烁"
            checked={prefs.cursorBlink}
            onchange={(value) => saveTerminalPrefs({ cursorBlink: value })}
          />
        </label>
        <div class="flex items-center justify-between text-xs">
          <span class="text-muted">滚动缓冲</span>
          <select
            class="rounded border border-line bg-surface px-1.5 py-0.5"
            value={prefs.scrollback}
            onchange={(event) =>
              saveTerminalPrefs({ scrollback: Number(event.currentTarget.value) })}
          >
            {#each scrollbackOptions as option (option)}
              <option value={option}>{option} 行</option>
            {/each}
          </select>
        </div>
      </div>
    {/if}
  </div>

  <!-- 拖拽影像：浮起标签卡片，垂直锚定标签条中心、水平跟随鼠标
       （left 由 pointermove 直接写 style，不走状态更新）。 -->
  {#if ghost}
    <div
      bind:this={ghostEl}
      class="pointer-events-none fixed z-50 flex max-w-44 -translate-y-1/2 scale-105 items-center gap-1.5 rounded-md border border-accent bg-card px-2.5 py-1 text-xs text-fg opacity-90 shadow-xl"
      style="top: {ghost.y}px;"
    >
      {#if ghost.pinned}
        <span class="shrink-0 text-[10px] text-accent">📌</span>
      {/if}
      <span class="truncate">{ghost.title}</span>
    </div>
  {/if}

  <div class="relative min-h-0 flex-1">
    {#each tabs as tab (tab.id)}
      <div class="absolute inset-0 {activeId === tab.id ? '' : 'invisible'}">
        {#if tab.error !== null}
          <div class="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
            <p class="text-sm text-fg">终端启动失败</p>
            <p class="max-w-md text-xs leading-5 text-muted">{tab.error}</p>
            <button
              class="rounded-md bg-accent-soft px-3 py-1.5 text-xs text-fg transition-colors hover:bg-accent-soft/70"
              onclick={() => retry(tab)}
            >
              重试
            </button>
          </div>
        {:else}
          <TerminalPane
            id={tab.id}
            active={activeId === tab.id}
            {prefs}
            shell={tab.shell}
            initialHistory={tab.restoreFrom !== null ? tab.restoreHistory : ''}
            onExit={onPaneExit}
            onTitle={onPaneTitle}
            onCwd={onPaneCwd}
            onBind={onPaneBind}
          />
        {/if}
      </div>
    {/each}
    {#if tabs.length === 0}
      <div class="flex h-full flex-col items-center justify-center gap-2 text-sm text-muted">
        <p>没有终端会话。点 + 新建。</p>
        {#if standalone}
          <p class="text-xs">主窗的标签可右键「分离到独立窗口」转移到这里。</p>
        {:else}
          <p class="text-xs">标签右键可分离到独立窗口。</p>
        {/if}
      </div>
    {/if}
  </div>
</section>

<ConfirmDialog
  open={closeTarget !== null}
  title="关闭终端"
  message={closeTarget
    ? `「${closeTarget.title}」仍在运行，关闭将结束该 shell 的进程${closeTarget.pinned !== null ? '，其固定（PIN）记录也会删除' : ''}。确定？`
    : ''}
  confirmLabel="结束进程"
  danger
  onconfirm={confirmClose}
  oncancel={() => (closeTarget = null)}
/>
