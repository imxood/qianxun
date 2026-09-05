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
  import { shellTitle } from '../../lib/utils/shell';
  import ConfirmDialog from '../../components/ConfirmDialog.svelte';
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

  const paneApis = new SvelteMap<number, PaneApi>();

  const prefs = $derived(
    settings.current?.terminal ?? {
      shell: 'auto',
      fontSize: 13,
      scrollback: 5000,
    },
  );

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
          title: shellTitle(info.shell),
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
    if (tab && title.trim() && !tab.manualTitle) tab.title = title;
  }

  function onPaneCwd(id: number, cwd: string): void {
    const tab = tabs.find((item) => item.id === id);
    if (tab) tab.cwd = cwd;
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
  const tabEls = new SvelteMap<number, HTMLElement>();

  function tabRef(node: HTMLElement, id: number): { destroy(): void } {
    tabEls.set(id, node);
    return {
      destroy() {
        tabEls.delete(id);
      },
    };
  }

  function tabPointerDown(event: PointerEvent, tab: Tab): void {
    if (event.button !== 0) return;
    // × 关闭钮自带点击语义，不作为拖拽起点。
    if ((event.target as HTMLElement).closest('[data-no-drag]')) return;
    dragId = tab.id;
    dragMoved = false;
    dragStartX = event.clientX;
    dragStartY = event.clientY;
    (event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId);
  }

  function tabPointerMove(event: PointerEvent): void {
    if (dragId === null) return;
    if (!dragMoved) {
      const dx = event.clientX - dragStartX;
      const dy = event.clientY - dragStartY;
      if (Math.hypot(dx, dy) < 5) return;
      dragMoved = true;
    }
    const from = tabs.findIndex((tab) => tab.id === dragId);
    if (from < 0) return;
    const target = insertionIndex(event.clientX);
    // 目标位与当前位相同（含 ±1 边界等价）则不动。
    if (target === from || target === from + 1) return;
    const next = [...tabs];
    const [moved] = next.splice(from, 1);
    next.splice(from < target ? target - 1 : target, 0, moved!);
    tabs = next;
  }

  /** 指针 x 落点对应的插入下标（标签中点为界；条末/越界 = 尾部）。 */
  function insertionIndex(x: number): number {
    for (let index = 0; index < tabs.length; index++) {
      const el = tabEls.get(tabs[index]!.id);
      if (!el) continue;
      const rect = el.getBoundingClientRect();
      if (x < rect.left + rect.width / 2) return index;
    }
    return tabs.length;
  }

  function tabPointerUp(event: PointerEvent, tab: Tab): void {
    if (dragId !== tab.id) return;
    suppressClick = dragMoved;
    dragId = null;
    dragMoved = false;
    const button = event.currentTarget as HTMLButtonElement;
    if (button.hasPointerCapture(event.pointerId)) {
      button.releasePointerCapture(event.pointerId);
    }
  }

  function tabClick(tab: Tab): void {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    activeId = tab.id;
  }
</script>

<section class="flex h-full flex-col overflow-hidden">
  <div
    class="flex select-none items-center gap-1 border-b border-line bg-surface px-2 py-1.5"
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
            ? 'relative z-10 opacity-70 ring-1 ring-accent shadow-lg'
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
          title="{tab.title}{tab.pinned !== null
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
    <button
      class="rounded-md px-2 py-1 text-sm text-muted transition-colors hover:bg-accent-soft hover:text-fg"
      title="新建终端"
      data-testid="terminal-new"
      onclick={() => void newTab()}
    >
      +
    </button>
  </div>

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
