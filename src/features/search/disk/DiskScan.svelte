<script module lang="ts">
  /**
   * 扫描树的一条面包屑：完整树根 + 扫描时间 + 可信度标注。
   * partial 非空 = 数字只是已扫描部分（被停止/出错）；skipped = 权限跳过数。
   * （两个 script 共享一个作用域：契约类型在实例 script 里统一导入。）
   */
  interface DiskTrailItem {
    entry: DiskEntry;
    /** 扫描完成时间（epoch ms）；0 = 占位视图还在扫描中。 */
    at: number;
    partial: { files: number; dirs: number } | null;
    skipped: number;
    largest: DiskLargeFile[];
  }

  /**
   * 会话级缓存（模块常驻）：切页/重进秒开上次扫描树，后台刷新兜底。
   * 不落盘——桌面端重启后重扫一次可接受，避免 localStorage 塞大 JSON。
   */
  const session: { home: DiskHome | null; trail: DiskTrailItem[] | null } = {
    home: null,
    trail: null,
  };

  const VIEW_KEY = 'qx-disk-view';

  function loadSavedView(): 'blocks' | 'list' {
    try {
      return localStorage.getItem(VIEW_KEY) === 'list' ? 'list' : 'blocks';
    } catch {
      return 'blocks';
    }
  }
</script>

<script lang="ts">
  /**
   * 磁盘扫描（文件页第三签）：目录占用分析 + 回收站清理。
   * 流式扫描「边扫边长」（进度帧携带根直接子项部分占用）；停止/出错留
   * 「部分结果」角标；会话缓存秒开 + 后台刷新；列表多选批量清理、
   * 受管目录保护警告、TOP 大文件直达、键盘导航、treemap 悬浮详情。
   */
  import { onMount } from 'svelte';
  import { Channel } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener';
  import { call } from '../../../lib/ipc';
  import { contextMenu } from '../../../lib/menu.svelte';
  import type {
    DiskEntry,
    DiskHome,
    DiskLargeFile,
    DiskPartialChild,
    DiskScanEvent,
    DiskScanProgress,
  } from '../../../lib/ipc/contract';
  import { formatBytes } from '../format';
  import { squarify, type Rect } from './treemap';

  // ---- 状态 --------------------------------------------------------------
  let home = $state<DiskHome | null>(null);
  /** 面包屑扫描链：trail 末项即 current。每项都是一次流式扫描的完整树根。 */
  let trail = $state<DiskTrailItem[]>([]);
  /** 流式扫描进行中（progress 帧到达中，Done 未到）。 */
  let scanning = $state(false);
  /** 正在扫描的目录（区分「浏览旧数据」与「新扫描目标」）。 */
  let scanningRoot = $state('');
  /** 本次扫描是否后台刷新（SWR：不遮暗旧数据、不逐帧生长，Done 时整体换新）。 */
  let background = $state(false);
  /** 最近一帧进度（后端 ~100ms 一帧）：实时数字与「边扫边长」数据源。 */
  let progress = $state<DiskScanProgress | null>(null);
  let stopping = $state(false);
  let view = $state<'blocks' | 'list'>(loadSavedView());
  let actionError = $state('');
  /** 待清理目标（单项或多选批量），非空时显示确认弹窗。 */
  let cleanTargets = $state<DiskEntry[]>([]);
  let cleaning = $state(false);
  /** 列表多选（路径）。 */
  let selected = $state<string[]>([]);
  let sortKey = $state<'size' | 'name'>('size');
  let sortDir = $state<1 | -1>(-1);
  /** 列表视图展开过「其余 N 项」的目录。 */
  let expanded = $state<string[]>([]);
  /** treemap 键盘焦点块。 */
  let focusKey = $state('');
  /** treemap 悬浮详情（跟随光标）。 */
  let tip = $state<{ entry: DiskEntry; parentSize: number; x: number; y: number } | null>(null);
  let toast = $state<{ text: string; action?: { label: string; run: () => void } } | null>(null);
  /** 用户添加过的外部目录（本地记忆，最多 6 个）。 */
  let externals = $state<string[]>(loadExternals());
  /** 分钟级心跳：驱动「X 分钟前扫描」相对时间刷新。 */
  let now = $state(Date.now());

  const EXTERNALS_KEY = 'qx-disk-externals';
  /** 单层展示上限：超出聚合为「其余 N 项」，C 盘级目录也不会撑爆 DOM。 */
  const DISPLAY_LIMIT = 300;

  function loadExternals(): string[] {
    try {
      const raw = localStorage.getItem(EXTERNALS_KEY);
      const parsed: unknown = raw ? JSON.parse(raw) : [];
      return Array.isArray(parsed)
        ? parsed.filter((item): item is string => typeof item === 'string')
        : [];
    } catch {
      return [];
    }
  }

  function saveExternals(): void {
    try {
      localStorage.setItem(EXTERNALS_KEY, JSON.stringify(externals));
    } catch {
      /* 隐私模式等：记忆失败无碍 */
    }
  }

  const current = $derived(trail.length > 0 ? (trail[trail.length - 1] ?? null) : null);
  const busy = $derived(scanning || cleaning || stopping);
  /** 前台扫描遮暗旧数据；后台刷新保持全亮。 */
  const dimClass = $derived(scanning && !background ? 'opacity-40' : '');
  const expandedHere = $derived(current !== null && expanded.includes(current.entry.path));

  // ---- 工具 --------------------------------------------------------------
  function normPath(path: string): string {
    return path.replace(/[\\/]+$/, '').toLowerCase();
  }

  function label(entry: DiskEntry): string {
    return home?.labels[entry.name] ?? entry.name;
  }

  function pathTail(path: string): string {
    const trimmed = path.replace(/[\\/]+$/, '');
    return trimmed.split(/[\\/]/).pop() ?? trimmed;
  }

  function agoText(at: number): string {
    if (!at) return '扫描中…';
    const diff = Math.max(0, now - at);
    if (diff < 45_000) return '刚刚扫描';
    if (diff < 3_600_000) return `${Math.round(diff / 60_000)} 分钟前扫描`;
    if (diff < 86_400_000) return `${Math.round(diff / 3_600_000)} 小时前扫描`;
    return `${Math.round(diff / 86_400_000)} 天前扫描`;
  }

  function percentOfParent(entry: DiskEntry): number {
    const parent = current?.entry;
    if (!parent || parent.size <= 0) return 0;
    return (entry.size / parent.size) * 100;
  }

  function percentText(entry: DiskEntry): string {
    const percent = percentOfParent(entry);
    return percent >= 10 ? `${Math.round(percent)}%` : `${percent.toFixed(1)}%`;
  }

  function barWidth(entry: DiskEntry): number {
    const percent = percentOfParent(entry);
    return percent > 0 ? Math.max(1, percent) : 0;
  }

  /** 受管目录：数据根直接子项里被千寻起过友好名的（清理会破坏功能）。 */
  function isManaged(entry: DiskEntry): boolean {
    if (!home) return false;
    const trimmed = entry.path.replace(/[\\/]+$/, '');
    const parent = trimmed.replace(/[\\/][^\\/]*$/, '');
    return (
      parent.length > 0 && normPath(parent) === normPath(home.root) && entry.name in home.labels
    );
  }

  // ---- 流式扫描（代际防竞态 + 边扫边长）------------------------------------
  let seq = 0;
  type ScanMode = 'reset' | 'push' | 'refresh';
  let lastLive = 0;

  function provisionalItem(path: string): DiskTrailItem {
    return {
      entry: { name: pathTail(path) || path, path, size: 0, dir: true, children: [] },
      at: 0,
      partial: null,
      skipped: 0,
      largest: [],
    };
  }

  function toEntry(child: DiskPartialChild): DiskEntry {
    return { name: child.name, path: child.path, size: child.size, dir: child.dir, children: [] };
  }

  /**
   * 边扫边长：进度帧的「根直接子项部分占用」逐帧长进当前层。
   * 仅前台扫描生效（后台刷新保持旧快照稳定，Done 时整体换新）。
   */
  function liveGrow(frame: DiskScanProgress): void {
    const top = trail[trail.length - 1];
    if (!top || normPath(top.entry.path) !== normPath(frame.root)) return;
    const stamp = Date.now();
    if (stamp - lastLive < 200) return; // 最多 5fps 重排，足够顺滑不空烧
    lastLive = stamp;
    top.entry.children = frame.topChildren.map(toEntry);
    top.entry.size = frame.bytes;
  }

  /**
   * 启动一次流式扫描：后端 rayon 并行遍历，progress 帧固定 ~100ms 一帧，
   * Done 帧携带完整树。换目标时后端自动作废旧扫描（rotate 语义）。
   * reset/push 立即切到占位视图逐帧生长；refresh 原地生长；background 仅收尾换新。
   */
  function scan(path: string, mode: ScanMode, opts: { background?: boolean } = {}): void {
    const ticket = ++seq;
    background = opts.background === true;
    scanning = true;
    stopping = false;
    actionError = '';
    progress = null;
    scanningRoot = path;
    lastLive = 0;
    focusKey = '';
    tip = null;
    if (!background) selected = [];

    if (mode === 'reset') {
      trail = [provisionalItem(path)];
    } else if (mode === 'push') {
      trail = [...trail, provisionalItem(path)];
    }

    const onEvent = new Channel<DiskScanEvent>();
    onEvent.onmessage = (event) => {
      if (ticket !== seq) return; // 过期帧丢弃（用户已换目标）。
      if (event.type === 'progress') {
        progress = event;
        if (!background) liveGrow(event);
        return;
      }
      if (!event.tree.path) {
        // 硬错误（后端已落日志）：收尾并给出可见反馈，占位视图撤掉。
        scanning = false;
        progress = null;
        scanningRoot = '';
        stopping = false;
        background = false;
        if (mode !== 'reset') trail = trail.slice(0, -1);
        actionError = '扫描没有完成，请重试或查看「环境」页日志。';
        return;
      }
      // Done：完整树快照，整体替换（含部分结果/跳过标注）。
      const item: DiskTrailItem = {
        entry: event.tree,
        at: Date.now(),
        partial: event.cancelled ? { files: event.files, dirs: event.dirs } : null,
        skipped: event.skipped,
        largest: event.largestFiles,
      };
      trail = mode === 'reset' ? [item] : [...trail.slice(0, -1), item];
      scanning = false;
      progress = null;
      scanningRoot = '';
      stopping = false;
      background = false;
      tip = null;
    };

    void call('disk_scan_stream', { path, onEvent }).catch((error) => {
      if (ticket !== seq) return;
      scanning = false;
      progress = null;
      scanningRoot = '';
      stopping = false;
      background = false;
      if (mode === 'push') trail = trail.slice(0, -1);
      actionError = error instanceof Error ? error.message : String(error);
    });
  }

  /** 停止当前扫描：后端置位取消标志，Done{cancelled:true} 帧照常收尾。 */
  async function stopScan(): Promise<void> {
    if (!scanning || stopping) return;
    stopping = true;
    try {
      await call('disk_scan_stop');
    } catch {
      stopping = false;
    }
  }

  /** 在已完成的树中按路径查找节点（下钻优先走本地数据，零 IO）。 */
  function findInTree(root: DiskEntry | null, path: string): DiskEntry | null {
    if (!root) return null;
    const target = normPath(path);
    const walk = (node: DiskEntry): DiskEntry | null => {
      if (normPath(node.path) === target) return node;
      for (const child of node.children) {
        const found = walk(child);
        if (found) return found;
      }
      return null;
    };
    return walk(root);
  }

  /**
   * 下钻（用户任意时刻可点）：
   * - 目标已在当前树里 → 直接本地导航，即时且零扫描（继承可信度标注）；
   * - 目标不在树里（首扫未完成/目录是新增的）→ 以它为根发起新流式扫描，
   *   旧扫描被后端静默作废——用户操作永远以最后一次点击为准。
   */
  function drill(entry: DiskEntry): void {
    if (!entry.dir || !entry.path) return;
    const from = trail[trail.length - 1];
    const local = findInTree(from?.entry ?? null, entry.path);
    focusKey = '';
    tip = null;
    if (from && local) {
      seq++; // 本地下钻同样使在途帧过期。
      trail = [
        ...trail,
        {
          entry: local,
          at: from.at,
          partial: from.partial,
          skipped: from.skipped,
          largest: from.largest,
        },
      ];
      progress = null;
      actionError = '';
      return;
    }
    scan(entry.path, 'push');
  }

  function crumbTo(index: number): void {
    if (index === trail.length - 1) return;
    seq++; // 回退是纯本地导航，丢弃在途帧。
    trail = trail.slice(0, index);
    progress = null;
    actionError = '';
    focusKey = '';
    tip = null;
  }

  function goHome(): void {
    if (home) scan(home.root, 'reset');
  }

  onMount(() => {
    const heartbeat = setInterval(() => (now = Date.now()), 30_000);
    void (async () => {
      try {
        if (session.trail && session.trail.length > 0) {
          // 秒开上次结果，再后台刷新当前层（SWR）。
          home = session.home ?? (await call<DiskHome>('disk_home'));
          trail = session.trail;
          const top = trail[trail.length - 1];
          if (top) scan(top.entry.path, 'refresh', { background: true });
        } else {
          home = await call<DiskHome>('disk_home');
          scan(home.root, 'reset');
        }
      } catch (error) {
        actionError = error instanceof Error ? error.message : String(error);
      }
    })();
    return () => clearInterval(heartbeat);
  });

  // 缓存随状态更新（切页销毁后仍在模块里，回来即秒开）。
  $effect(() => {
    session.home = home;
    session.trail = trail.length > 0 ? trail : null;
  });

  async function pickExternal(): Promise<void> {
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== 'string' || !picked.trim()) return;
    externals = [picked, ...externals.filter((item) => item !== picked)].slice(0, 6);
    saveExternals();
    scan(picked, 'reset');
  }

  function removeExternal(path: string, event: MouseEvent): void {
    event.stopPropagation();
    externals = externals.filter((item) => item !== path);
    saveExternals();
  }

  // ---- 清理（确认后走回收站；支持批量；受管目录警告）------------------------
  function askClean(entry: DiskEntry): void {
    if (!entry.path) return;
    cleanTargets = [entry];
  }

  function toggleSelect(path: string): void {
    selected = selected.includes(path)
      ? selected.filter((item) => item !== path)
      : [...selected, path];
  }

  function toggleSelectAll(): void {
    selected = allSelected ? [] : visibleChildren.filter((entry) => entry.path).map((e) => e.path);
  }

  function askCleanSelected(): void {
    if (selectedEntries.length === 0) return;
    cleanTargets = [...selectedEntries];
  }

  async function confirmClean(): Promise<void> {
    if (cleanTargets.length === 0 || !current) return;
    cleaning = true;
    actionError = '';
    const refreshPath = current.entry.path;
    let freed = 0;
    let failed = 0;
    try {
      for (const target of cleanTargets) {
        try {
          await call('disk_clean', { path: target.path });
          freed += target.size;
        } catch {
          failed += 1;
        }
      }
      cleanTargets = [];
      selected = [];
      if (freed > 0) {
        showToast(`已释放 ${formatBytes(freed)}${failed > 0 ? ` · ${failed} 项失败` : ''}`, {
          label: '查看回收站',
          run: () => void openPath('shell:RecycleBinFolder').catch(() => {}),
        });
      } else if (failed > 0) {
        actionError = `${failed} 项清理失败，请检查权限后重试。`;
      }
      scan(refreshPath, 'refresh');
    } finally {
      cleaning = false;
    }
  }

  // ---- 右键菜单 ------------------------------------------------------------
  function menuFor(event: MouseEvent, entry: DiskEntry): void {
    if (!entry.path) return;
    event.preventDefault();
    const items: Array<{ label: string; onclick: () => void }> = [];
    if (entry.dir) items.push({ label: '进入', onclick: () => drill(entry) });
    items.push({
      label: '在资源管理器中显示',
      onclick: () => void revealItemInDir(entry.path).catch(() => {}),
    });
    items.push({ label: '移入回收站…', onclick: () => askClean(entry) });
    contextMenu.show(event, items);
  }

  // ---- 排序 / 展开 ----------------------------------------------------------
  /** 视图切换：记忆到本地，并清掉与视图绑定的选择/焦点/悬浮态。 */
  function setView(next: 'blocks' | 'list'): void {
    if (view === next) return;
    view = next;
    selected = [];
    focusKey = '';
    tip = null;
    try {
      localStorage.setItem(VIEW_KEY, next);
    } catch {
      /* 隐私模式等：记忆失败无碍 */
    }
  }

  function setSort(key: 'name' | 'size'): void {
    if (sortKey === key) {
      sortDir = sortDir === 1 ? -1 : 1;
    } else {
      sortKey = key;
      sortDir = key === 'name' ? 1 : -1;
    }
  }

  function sortMark(key: 'name' | 'size'): string {
    if (sortKey !== key) return '';
    return sortDir === 1 ? ' ↑' : ' ↓';
  }

  function toggleExpand(): void {
    const path = current?.entry.path;
    if (!path) return;
    expanded = expanded.includes(path)
      ? expanded.filter((item) => item !== path)
      : [...expanded, path];
  }

  /** 展示截断：巨目录聚合为「其余 N 项」占位；列表可展开全部。 */
  const sortedChildren = $derived.by<DiskEntry[]>(() => {
    const entry = current?.entry;
    if (!entry) return [];
    const list = [...entry.children];
    if (sortKey === 'name') {
      list.sort((a, b) => sortDir * a.name.localeCompare(b.name));
    } else {
      list.sort((a, b) => sortDir * (a.size - b.size) || a.name.localeCompare(b.name));
    }
    return list;
  });

  const visibleChildren = $derived.by<DiskEntry[]>(() => {
    const entry = current?.entry;
    if (!entry) return [];
    const limit =
      view === 'list' && expandedHere
        ? sortedChildren.length
        : Math.min(sortedChildren.length, DISPLAY_LIMIT);
    const head = sortedChildren.slice(0, limit);
    const restCount = sortedChildren.length - head.length;
    if (restCount > 0) {
      const restSize = sortedChildren.slice(head.length).reduce((sum, item) => sum + item.size, 0);
      return [
        ...head,
        { name: `其余 ${restCount} 项`, path: '', size: restSize, dir: false, children: [] },
      ];
    }
    return head;
  });

  /** 当前层的最大文件（下钻后按子树前缀过滤，仍是这层的答案）。 */
  const topFiles = $derived.by<DiskLargeFile[]>(() => {
    const item = current;
    if (!item) return [];
    const prefix = normPath(item.entry.path);
    return item.largest
      .filter((file) => {
        const path = normPath(file.path);
        // 段边界匹配：C:\root 不能误吞兄弟目录 C:\root2 的文件。
        return (
          path.startsWith(prefix) &&
          (path.length === prefix.length ||
            path[prefix.length] === '\\' ||
            path[prefix.length] === '/')
        );
      })
      .slice(0, 5);
  });

  // ---- 列表多选（依赖 visibleChildren，置于其声明后）-------------------------
  const selectedEntries = $derived(
    visibleChildren.filter((entry) => entry.path && selected.includes(entry.path)),
  );
  const selectedSize = $derived(selectedEntries.reduce((sum, entry) => sum + entry.size, 0));
  const allSelected = $derived(
    selectedEntries.length > 0 &&
      visibleChildren.some((entry) => entry.path) &&
      selectedEntries.length === visibleChildren.filter((entry) => entry.path).length,
  );

  /** 批量清理里的受管目录（确认弹窗给保护警告）。 */
  const managedTargets = $derived(cleanTargets.filter((entry) => isManaged(entry)));

  // ---- treemap 布局 ---------------------------------------------------------
  let box = $state<HTMLDivElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  let boxSize = $state({ w: 0, h: 0 });

  $effect(() => {
    if (!box) return;
    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) boxSize = { w: rect.width, h: rect.height };
    });
    observer.observe(box);
    return () => observer.disconnect();
  });

  interface Block {
    entry: DiskEntry;
    rect: Rect;
    mix: number;
  }

  const blocks = $derived.by<Block[]>(() => {
    const entry = current?.entry;
    if (!entry || view !== 'blocks' || boxSize.w <= 0 || boxSize.h <= 0) return [];
    const total = entry.size;
    const rects = squarify(
      visibleChildren.map((item) => Math.max(item.size, 1)),
      { x: 0, y: 0, w: boxSize.w, h: boxSize.h },
    );
    return visibleChildren.map((item, index) => {
      const share = total > 0 ? item.size / total : 0;
      const mix = item.dir ? Math.round(10 + 55 * Math.sqrt(share)) : 0;
      return { entry: item, rect: rects[index]!, mix };
    });
  });

  function blockStyle(block: Block): string {
    const pad = 1; // 方块间 2px 缝（相邻各让 1px），缝色即容器底色。
    const { x, y, w, h } = block.rect;
    const fill = block.entry.dir
      ? `color-mix(in oklab, var(--qx-accent) ${block.mix}%, var(--qx-surface))`
      : block.entry.path
        ? 'var(--qx-surface)'
        : 'transparent';
    return [
      `left:${x + pad}px`,
      `top:${y + pad}px`,
      `width:${Math.max(0, w - pad * 2)}px`,
      `height:${Math.max(0, h - pad * 2)}px`,
      `background:${fill}`,
    ].join(';');
  }

  function showLabel(rect: Rect): boolean {
    return rect.w >= 72 && rect.h >= 34;
  }

  // ---- treemap 键盘导航 ------------------------------------------------------
  function blockKeyOf(entry: DiskEntry): string {
    return entry.path || entry.name;
  }

  const ARROWS: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, -1],
    ArrowDown: [0, 1],
  };

  /** 方向键几何选邻：朝目标方向前进 + 垂直偏移平方惩罚（走直线优先）。 */
  function pickBlock(from: Block | null, dx: number, dy: number): Block | null {
    const candidates = blocks.filter((block) => block.entry.path);
    if (candidates.length === 0) return null;
    if (!from) return candidates[0] ?? null;
    const cx = from.rect.x + from.rect.w / 2;
    const cy = from.rect.y + from.rect.h / 2;
    let best: Block | null = null;
    let bestScore = Infinity;
    for (const block of candidates) {
      if (block.entry.path === from.entry.path) continue;
      const bx = block.rect.x + block.rect.w / 2;
      const by = block.rect.y + block.rect.h / 2;
      const forward = (bx - cx) * dx + (by - cy) * dy;
      if (forward <= 0) continue;
      const side = Math.abs((bx - cx) * dy) + Math.abs((by - cy) * dx);
      const score = forward + side * side * 4;
      if (score < bestScore) {
        bestScore = score;
        best = block;
      }
    }
    return best;
  }

  function setBlockFocus(block: Block): void {
    focusKey = blockKeyOf(block.entry);
    const el = box?.querySelector<HTMLButtonElement>(`[data-key="${window.CSS.escape(focusKey)}"]`);
    el?.focus();
  }

  function focusedBlock(): Block | null {
    return blocks.find((block) => blockKeyOf(block.entry) === focusKey) ?? null;
  }

  function onTreemapKeydown(event: KeyboardEvent): void {
    const delta = ARROWS[event.key];
    if (delta) {
      event.preventDefault();
      const next = pickBlock(focusedBlock(), delta[0], delta[1]);
      if (next) setBlockFocus(next);
      return;
    }
    if (event.key === 'Enter') {
      const from = focusedBlock();
      if (from?.entry.path) {
        event.preventDefault();
        drill(from.entry);
      }
      return;
    }
    if (event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      const candidates = blocks.filter((block) => block.entry.path);
      const target =
        event.key === 'Home' ? candidates[0] : (candidates[candidates.length - 1] ?? null);
      if (target) setBlockFocus(target);
    }
  }

  /** 列表方向键在行间移动焦点（checkbox 不参与 Tab 链）。 */
  function onListKeydown(event: KeyboardEvent): void {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const rows = Array.from(listEl?.querySelectorAll<HTMLButtonElement>('button[data-row]') ?? []);
    if (rows.length === 0) return;
    const index = rows.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === 'ArrowDown'
        ? Math.min(rows.length - 1, index + 1)
        : Math.max(0, index === -1 ? 0 : index - 1);
    const target = rows[next];
    if (target && next !== index) {
      event.preventDefault();
      target.focus();
    }
  }

  // ---- 全局键盘：Esc 逐级退出（弹窗 > 选择 > 目录层级）-----------------------
  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape') return;
    const target = event.target as HTMLElement | null;
    if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA')) return;
    if (cleanTargets.length > 0) {
      cleanTargets = [];
      return;
    }
    if (selected.length > 0) {
      selected = [];
      return;
    }
    if (trail.length > 1) crumbTo(trail.length - 2);
  }

  // ---- treemap 悬浮详情 ------------------------------------------------------
  function placeTip(event: MouseEvent): { x: number; y: number } {
    const width = 320;
    const height = 110;
    let x = event.clientX + 14;
    let y = event.clientY + 16;
    if (x + width > window.innerWidth) x = event.clientX - width - 10;
    if (y + height > window.innerHeight) y = event.clientY - height - 10;
    return { x: Math.max(8, x), y: Math.max(8, y) };
  }

  function showTip(block: Block, event: MouseEvent): void {
    if (!block.entry.path) {
      tip = null;
      return;
    }
    const position = placeTip(event);
    tip = {
      entry: block.entry,
      parentSize: current?.entry.size ?? 0,
      x: position.x,
      y: position.y,
    };
  }

  function moveTip(event: MouseEvent): void {
    if (!tip) return;
    const position = placeTip(event);
    tip = { ...tip, x: position.x, y: position.y };
  }

  // ---- toast -----------------------------------------------------------------
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  function showToast(text: string, action?: { label: string; run: () => void }): void {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { text, action };
    toastTimer = setTimeout(() => {
      toast = null;
      toastTimer = null;
    }, 6_000);
  }

  function dismissToast(): void {
    if (toastTimer) {
      clearTimeout(toastTimer);
      toastTimer = null;
    }
    toast = null;
  }

  const crumbs = $derived(
    trail.map((item, index) => ({
      index,
      path: item.entry.path,
      label:
        index === 0 && home && item.entry.path === home.root
          ? '数据目录'
          : (home?.labels[item.entry.name] ?? item.entry.name),
    })),
  );

  /** 实时进度文案：扫描中它每 100ms 刷新一次，替代干等的「扫描中」。 */
  const liveLine = $derived.by<string>(() => {
    if (!progress) return '';
    const parts = [`已发现 ${progress.files.toLocaleString()} 个文件`];
    if (progress.dirs > 0) parts.push(`${progress.dirs.toLocaleString()} 个目录`);
    parts.push(formatBytes(progress.bytes));
    if (progress.skipped > 0) parts.push(`跳过 ${progress.skipped.toLocaleString()} 项`);
    return parts.join(' · ');
  });
</script>

<svelte:window onkeydown={onWindowKeydown} />

<section class="flex h-full flex-col gap-3">
  <!-- 路径行：面包屑 + 汇总 + 视图切换 + 动作 -->
  <div class="flex shrink-0 items-center gap-2">
    <nav class="flex min-w-0 flex-1 items-center gap-1 text-sm" aria-label="目录层级">
      {#each crumbs as crumb (crumb.path)}
        {#if crumb.index > 0}
          <span class="shrink-0 text-muted/60">/</span>
        {/if}
        <button
          class="max-w-44 truncate rounded px-1.5 py-0.5 transition-colors {crumb.index ===
          crumbs.length - 1
            ? 'font-medium text-fg'
            : 'text-muted hover:bg-accent-soft hover:text-fg'}"
          title={crumb.path}
          onclick={() => crumbTo(crumb.index)}
        >
          {crumb.label}
        </button>
      {/each}
      {#if current}
        <span class="ml-2 shrink-0 text-xs text-muted">
          {formatBytes(current.entry.size)} · {current.entry.children.length.toLocaleString()} 项 ·
          {agoText(current.at)}
        </span>
        {#if current.partial}
          <span
            class="shrink-0 rounded bg-warning/15 px-1.5 py-0.5 text-[11px] text-warning"
            title="上次扫描被停止，数字只包含已扫描部分"
          >
            部分结果
          </span>
        {/if}
      {/if}
    </nav>

    <div class="flex shrink-0 items-center gap-1 rounded-md border border-line bg-surface p-0.5">
      <button
        class="rounded px-2 py-1 text-xs transition-colors {view === 'blocks'
          ? 'bg-accent-soft font-medium text-fg'
          : 'text-muted hover:text-fg'}"
        onclick={() => setView('blocks')}
      >
        方块
      </button>
      <button
        class="rounded px-2 py-1 text-xs transition-colors {view === 'list'
          ? 'bg-accent-soft font-medium text-fg'
          : 'text-muted hover:text-fg'}"
        onclick={() => setView('list')}
      >
        列表
      </button>
    </div>
    <button
      class="qx-btn qx-btn-outline qx-btn-sm h-7"
      disabled={cleaning}
      title="重新扫描当前目录"
      onclick={() => current && scan(current.entry.path, 'refresh')}
    >
      重新扫描
    </button>
    <button class="qx-btn qx-btn-primary qx-btn-sm h-7" onclick={() => void pickExternal()}>
      添加目录
    </button>
    {#if scanning}
      <button
        class="qx-btn qx-btn-danger-outline qx-btn-sm h-7"
        disabled={stopping}
        data-testid="disk-scan-stop"
        onclick={() => void stopScan()}
      >
        {stopping ? '停止中…' : '停止'}
      </button>
    {/if}
  </div>

  {#if externals.length > 0}
    <div class="flex shrink-0 flex-wrap items-center gap-1.5">
      {#each externals as external (external)}
        <span
          class="group flex items-center gap-1 rounded-full border border-line bg-surface py-0.5 pl-2.5 pr-1 text-xs text-fg transition-colors hover:border-accent"
        >
          <button
            class="max-w-48 truncate"
            title={external}
            onclick={() => scan(external, 'reset')}
          >
            {pathTail(external)}
          </button>
          <button
            class="rounded-full px-1 text-muted transition-colors hover:text-danger"
            aria-label="移除记录 {pathTail(external)}"
            onclick={(event) => removeExternal(external, event)}
          >
            ×
          </button>
        </span>
      {/each}
    </div>
  {/if}

  {#if actionError}
    <p class="shrink-0 text-sm text-danger">{actionError}</p>
  {/if}

  <!-- 实时进度行：扫描期间固定频率刷新，不是干等的「扫描中」 -->
  {#if scanning}
    <div
      class="flex shrink-0 items-center gap-2 text-xs text-muted"
      data-testid="disk-scan-progress"
      role="status"
    >
      <span
        class="inline-block size-3 shrink-0 animate-spin rounded-full border-2 border-line border-t-accent"
        aria-hidden="true"
      ></span>
      <span class="font-mono">{liveLine || '正在启动扫描…'}</span>
      <span class="truncate text-muted/60">（{background ? '后台刷新 · ' : ''}{scanningRoot}）</span
      >
    </div>
  {/if}

  <!-- 批量清理条：列表多选后出现 -->
  {#if view === 'list' && selectedEntries.length > 0}
    <div
      class="flex shrink-0 items-center gap-2 rounded-lg border border-accent/40 bg-accent-soft/60 px-3 py-1.5 text-xs"
    >
      <span class="font-medium text-fg">
        已选 {selectedEntries.length} 项 · {formatBytes(selectedSize)}
      </span>
      <button
        class="qx-btn qx-btn-danger qx-btn-sm h-6"
        disabled={cleaning}
        onclick={askCleanSelected}
      >
        移入回收站
      </button>
      <button class="qx-btn qx-btn-ghost qx-btn-sm h-6" onclick={() => (selected = [])}>
        取消选择
      </button>
    </div>
  {/if}

  <!-- 主体 -->
  <div class="qx-card relative min-h-0 flex-1 overflow-hidden bg-bg">
    {#if !current}
      <p class="flex h-full items-center justify-center text-sm text-muted">准备扫描…</p>
    {:else if current.entry.children.length === 0 && !scanning}
      <p class="flex h-full items-center justify-center text-sm text-muted">空目录</p>
    {:else if view === 'blocks'}
      <div
        bind:this={box}
        class="absolute inset-0 outline-none transition-opacity {dimClass}"
        data-testid="disk-treemap"
        tabindex="0"
        role="application"
        aria-label="占用方块图：方向键移动焦点，回车进入目录，Esc 返回上一级"
        onkeydown={onTreemapKeydown}
      >
        {#each blocks as block (block.entry.path || block.entry.name)}
          <button
            class="group absolute overflow-hidden rounded-[3px] text-left transition-[filter] {block
              .entry.path
              ? 'hover:z-10 hover:ring-2 hover:ring-accent hover:brightness-105'
              : 'border border-dashed border-line'} {focusKey === blockKeyOf(block.entry)
              ? 'z-10 ring-2 ring-accent'
              : ''}"
            tabindex="-1"
            data-key={blockKeyOf(block.entry)}
            style={blockStyle(block)}
            aria-label="{label(block.entry)}，占用 {formatBytes(block.entry.size)}"
            onclick={() => drill(block.entry)}
            oncontextmenu={(event) => menuFor(event, block.entry)}
            onmouseenter={(event) => showTip(block, event)}
            onmousemove={(event) => moveTip(event)}
            onmouseleave={() => (tip = null)}
          >
            {#if showLabel(block.rect) && block.entry.path}
              <span
                class="pointer-events-none absolute inset-0 flex flex-col justify-between p-1.5"
              >
                <span class="truncate text-xs font-medium leading-4 text-fg">
                  {label(block.entry)}
                </span>
                <span class="text-[11px] leading-4 text-fg/70">
                  {formatBytes(block.entry.size)}
                </span>
              </span>
            {/if}
          </button>
        {/each}
      </div>
    {:else}
      <div
        bind:this={listEl}
        class="absolute inset-0 overflow-y-auto transition-opacity {dimClass}"
        data-testid="disk-list"
        onkeydown={onListKeydown}
      >
        <!-- 表头：全选 + 排序 -->
        <div
          class="sticky top-0 z-10 flex items-center gap-3 border-b border-line bg-card px-4 py-1.5 text-xs text-muted"
        >
          <input
            type="checkbox"
            class="size-3.5 shrink-0 accent-[var(--qx-accent)]"
            aria-label="全选本层可见项"
            checked={allSelected}
            onchange={toggleSelectAll}
          />
          <button
            class="flex-1 text-left transition-colors hover:text-fg"
            onclick={() => setSort('name')}
          >
            名称{sortMark('name')}
          </button>
          {#if expandedHere}
            <button
              class="shrink-0 text-muted transition-colors hover:text-fg"
              onclick={toggleExpand}
            >
              收起
            </button>
          {/if}
          <span class="w-12 shrink-0 text-right">占比</span>
          <button
            class="w-20 shrink-0 text-right transition-colors hover:text-fg"
            onclick={() => setSort('size')}
          >
            占用{sortMark('size')}
          </button>
        </div>
        <ul class="divide-y divide-line">
          {#each visibleChildren as entry (entry.path || entry.name)}
            {#if !entry.path}
              <!-- 「其余 N 项」占位：列表里可展开全部 -->
              <li>
                <button
                  class="flex w-full items-center justify-center gap-2 px-4 py-2.5 text-xs text-muted transition-colors hover:bg-accent-soft"
                  onclick={toggleExpand}
                >
                  {entry.name}（{formatBytes(entry.size)}）· 点击展开
                </button>
              </li>
            {:else}
              <li class="flex items-center gap-1 pr-4 transition-colors hover:bg-accent-soft">
                <input
                  type="checkbox"
                  class="size-3.5 shrink-0 accent-[var(--qx-accent)]"
                  aria-label="选择 {label(entry)}"
                  checked={selected.includes(entry.path)}
                  onclick={(event) => event.stopPropagation()}
                  onchange={() => toggleSelect(entry.path)}
                  tabindex="-1"
                />
                <button
                  class="flex min-w-0 flex-1 items-center gap-3 py-2 text-left text-sm"
                  data-row
                  onclick={() => drill(entry)}
                  oncontextmenu={(event) => menuFor(event, entry)}
                  aria-label="{label(entry)}，占用 {formatBytes(entry.size)}"
                >
                  <svg
                    viewBox="0 0 24 24"
                    class="size-4 shrink-0 text-muted"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="1.6"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    {#if entry.dir}
                      <path
                        d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2z"
                      />
                    {:else}
                      <path d="M7 3h7l4 4v14H7zM14 3v4h4" />
                    {/if}
                  </svg>
                  <span class="min-w-0 flex-1">
                    <span class="flex items-baseline gap-2">
                      <span class="truncate text-fg">{label(entry)}</span>
                      {#if entry.dir && entry.children.length > 0}
                        <span class="shrink-0 text-[11px] text-muted">
                          {entry.children.length} 项
                        </span>
                      {/if}
                    </span>
                    <span class="mt-1 block h-1 w-full overflow-hidden rounded-full bg-line">
                      <span
                        class="block h-full rounded-full bg-accent/70"
                        style={`width:${barWidth(entry)}%`}
                      ></span>
                    </span>
                  </span>
                  <span class="w-12 shrink-0 text-right font-mono text-[11px] text-muted">
                    {percentText(entry)}
                  </span>
                  <span class="w-20 shrink-0 text-right font-mono text-xs text-muted">
                    {formatBytes(entry.size)}
                  </span>
                </button>
              </li>
            {/if}
          {/each}
        </ul>
      </div>
    {/if}

    {#if busy && current}
      <span
        class="absolute right-3 top-3 flex items-center gap-1.5 rounded-full bg-card px-2.5 py-1 text-xs text-accent shadow"
        role="status"
      >
        <span
          class="inline-block size-3 animate-spin rounded-full border-2 border-line border-t-accent"
          aria-hidden="true"
        ></span>
        {cleaning ? '清理中…' : stopping ? '停止中…' : background ? '刷新中…' : '扫描中'}
      </span>
    {/if}
  </div>

  <!-- 底部：可信度脚注 + 最大文件直达 + 回根 -->
  <div class="flex shrink-0 flex-col gap-1.5">
    {#if current?.partial || (current && current.skipped > 0)}
      <p class="text-xs text-muted">
        {#if current?.partial}
          部分结果：仅含已发现的 {current.partial.files.toLocaleString()} 个文件 /
          {current.partial.dirs.toLocaleString()}
          个目录，重新扫描可补全。
        {/if}
        {#if current && current.skipped > 0}
          {#if current.partial}·
          {/if}另有 {current.skipped.toLocaleString()}
          项因权限或系统错误未统计
        {/if}
      </p>
    {/if}
    {#if topFiles.length > 0}
      <div class="flex flex-wrap items-center gap-1.5 text-xs text-muted">
        <span>最大文件</span>
        {#each topFiles as file (file.path)}
          <button
            class="max-w-64 truncate rounded-full border border-line bg-surface px-2 py-0.5 transition-colors hover:border-accent"
            title="{file.path} · 点击在资源管理器中显示"
            onclick={() => void revealItemInDir(file.path).catch(() => {})}
          >
            {file.name} · {formatBytes(file.size)}
          </button>
        {/each}
      </div>
    {/if}
    {#if home}
      <button
        class="self-start text-xs text-muted transition-colors hover:text-fg"
        onclick={goHome}
      >
        返回数据目录
      </button>
    {/if}
  </div>
</section>

<!-- treemap 悬浮详情：路径 + 占用 + 占比 + 子项数 -->
{#if tip}
  <div
    class="pointer-events-none fixed z-50 max-w-80 rounded-lg border border-white/15 bg-neutral-900/95 px-3 py-2 text-xs text-white shadow-xl"
    style="left:{tip.x}px; top:{tip.y}px;"
  >
    <p class="max-w-72 truncate font-medium">{label(tip.entry)}</p>
    <p class="mt-0.5 max-w-72 truncate font-mono text-[11px] text-white/55">{tip.entry.path}</p>
    <p class="mt-1 font-mono">
      {formatBytes(tip.entry.size)}
      {#if tip.parentSize > 0}
        <span class="text-white/70">· 占父级 {percentText(tip.entry)}</span>
      {/if}
    </p>
    {#if tip.entry.dir && tip.entry.children.length > 0}
      <p class="mt-0.5 text-white/70">{tip.entry.children.length.toLocaleString()} 个子项</p>
    {/if}
  </div>
{/if}

<!-- 轻提示：清理结果 + 回收站直达 -->
{#if toast}
  <div
    class="fixed bottom-6 left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-lg bg-neutral-900/95 px-4 py-2 text-xs text-white shadow-xl"
    role="status"
  >
    <span>{toast.text}</span>
    {#if toast.action}
      <button
        class="font-medium text-sky-300 transition-colors hover:text-sky-200"
        onclick={() => {
          const action = toast?.action;
          dismissToast();
          action?.run();
        }}
      >
        {toast.action.label}
      </button>
    {/if}
  </div>
{/if}

<!-- 清理确认：单项/批量共用；回收站可还原，受管目录额外警告 -->
{#if cleanTargets.length > 0}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6">
    <div
      class="qx-card w-full max-w-md p-5 shadow-xl"
      role="alertdialog"
      aria-modal="true"
      aria-label="确认移入回收站"
      data-testid="disk-clean-confirm"
    >
      <h2 class="text-sm font-medium">移入回收站</h2>
      {#if cleanTargets.length === 1}
        <p class="mt-3 break-all rounded-md bg-bg p-2 font-mono text-xs text-muted">
          {cleanTargets[0]?.path}
        </p>
        <p class="mt-3 text-sm text-fg">
          占用 {formatBytes(cleanTargets[0]?.size ?? 0)}，可在回收站还原。
        </p>
      {:else}
        <p class="mt-3 text-sm text-fg">
          已选 {cleanTargets.length} 项 · 共
          {formatBytes(cleanTargets.reduce((sum, entry) => sum + entry.size, 0))}，可在回收站还原。
        </p>
        <ul
          class="mt-3 max-h-36 space-y-1 overflow-y-auto rounded-md bg-bg p-2 font-mono text-xs text-muted"
        >
          {#each cleanTargets.slice(0, 8) as entry (entry.path)}
            <li class="truncate">{entry.path}</li>
          {/each}
          {#if cleanTargets.length > 8}
            <li>… 其余 {cleanTargets.length - 8} 项</li>
          {/if}
        </ul>
      {/if}
      {#if managedTargets.length > 0}
        <p class="mt-3 rounded-md border border-warning/40 bg-warning/10 p-2 text-xs text-warning">
          {managedTargets.map((entry) => home?.labels[entry.name] ?? entry.name).join('、')}
          是千寻运行所需目录，清理后相关功能不可用，需重新下载/安装才能恢复。
        </p>
      {/if}
      <div class="mt-5 flex justify-end gap-2">
        <button class="qx-btn qx-btn-outline qx-btn-md" onclick={() => (cleanTargets = [])}>
          取消
        </button>
        <button
          class="qx-btn qx-btn-danger qx-btn-md"
          disabled={cleaning}
          data-testid="disk-clean-confirm-button"
          onclick={() => void confirmClean()}
        >
          {cleaning
            ? '清理中…'
            : cleanTargets.length > 1
              ? `移入回收站（${cleanTargets.length} 项）`
              : '移入回收站'}
        </button>
      </div>
    </div>
  </div>
{/if}
