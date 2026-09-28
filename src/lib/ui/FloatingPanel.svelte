<script module lang="ts">
  /**
   * FloatingPanel —— 通用可拖动浮窗(全应用复用)。
   *
   * - 模块级 `zTop` 计数器实现点击置顶:pointerdown 时 `z = ++zTop`;
   *   `activePanel` 单值记录当前 active 浮窗(见 `getActivePanel`)。
   * - 持久化键统一由 `panelStorageKey(id)` 拼接,调用方只传 id。
   */
  const STORAGE_PREFIX = 'qx.mario.panel.';

  /** 拼出 localStorage 持久化键(导出便于测试与调用方对齐)。 */
  export function panelStorageKey(id: string): string {
    return `${STORAGE_PREFIX}${id}`;
  }

  // ---- 模块级 z 序与 active 记录:所有浮窗实例共享 ----
  let zTop = 100;
  let activePanel: string | null = null;
  let uidCounter = 0;

  /** 当前置顶(最后被点击)的浮窗标识,模块级单值。 */
  export function getActivePanel(): string | null {
    return activePanel;
  }

  /** 实例兜底唯一 id(无 persistKey/testid 时用)。 */
  function nextPanelUid(): string {
    return `panel-${++uidCounter}`;
  }
</script>

<script lang="ts">
  import { onMount } from 'svelte';

  import type { Snippet } from 'svelte';

  interface Props {
    /** 标题(超过 20 字符截断)。 */
    title: string;
    /** 容器 data-testid;派生出 -header/-fold/-close/-body。 */
    testid?: string;
    /** 宽度 px,默认 320;小视口(<900px)下退化为 min(96vw, width)。 */
    width?: number;
    /** 初始位置,默认视口居中;有持久化时持久化优先。 */
    initial?: { x: number; y: number } | 'center';
    /** 提供则把位置/折叠态持久化到 localStorage。 */
    persistKey?: string;
    /** 点 ✕ 时回调;组件不自我销毁,显隐由父级控制。 */
    onclose?: () => void;
    /** 正文内容。 */
    children: Snippet;
    /** 标题栏右侧动作插槽(关闭按钮之外)。 */
    headerActions?: Snippet;
  }

  let {
    title,
    testid,
    width = 320,
    initial = 'center',
    persistKey,
    onclose,
    children,
    headerActions,
  }: Props = $props();

  /** 拖拽/键盘移动后水平方向至少保留的可见宽度 px。 */
  const MIN_VISIBLE = 40;
  /** 标题栏聚焦时方向键步长 px。 */
  const KEY_STEP = 16;
  /** 低于此视口宽按小视口退化(宽 min(96vw, width)、初始底部居中)。 */
  const SMALL_VIEWPORT = 900;

  interface Pos {
    x: number;
    y: number;
  }

  interface PanelPersist {
    v: 1;
    x: number;
    y: number;
    folded: boolean;
  }

  const tid = (suffix: string): string | undefined => (testid ? `${testid}-${suffix}` : undefined);

  let panelEl: HTMLDivElement | null = null;

  // ---- 持久化:读写均 try/catch 静默(隐私模式/配额满不致命) ----
  const saved = readPersisted();

  function readPersisted(): PanelPersist | null {
    if (!persistKey) return null;
    try {
      const raw = localStorage.getItem(panelStorageKey(persistKey));
      if (!raw) return null;
      const p = JSON.parse(raw) as Partial<PanelPersist> | null;
      if (!p || p.v !== 1 || typeof p.x !== 'number' || typeof p.y !== 'number') return null;
      return { v: 1, x: p.x, y: p.y, folded: p.folded === true };
    } catch {
      return null;
    }
  }

  function writePersisted(): void {
    if (!persistKey) return;
    try {
      const value: PanelPersist = { v: 1, x: Math.round(pos.x), y: Math.round(pos.y), folded };
      localStorage.setItem(panelStorageKey(persistKey), JSON.stringify(value));
    } catch {
      /* 静默 */
    }
  }

  // ---- 位置状态:创建即分配 z 序并记为 active ----
  let small = $state(window.innerWidth < SMALL_VIEWPORT);
  let z = $state(++zTop);
  const fallbackId = nextPanelUid();
  const panelId = $derived(persistKey ?? testid ?? fallbackId);

  /** 把本实例记为模块级 active 浮窗。 */
  function recordActive(): void {
    activePanel = panelId;
  }

  recordActive(); // 创建即记为 active
  let folded = $state(saved?.folded ?? false);
  let pos = $state(initialPos());

  /** 小视口下实际宽度 px(挂载前用视口估算,挂载后优先实测)。 */
  function panelPxWidth(): number {
    if (panelEl && panelEl.offsetWidth > 0) return panelEl.offsetWidth;
    const vw = window.innerWidth;
    return small ? Math.min(vw * 0.96, width) : width;
  }

  /** 初始位置:持久化 > 小视口底部居中 > initial 居中/坐标。 */
  function initialPos(): Pos {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const w = vw < SMALL_VIEWPORT ? Math.min(vw * 0.96, width) : width;
    if (saved) return clampPos({ x: saved.x, y: saved.y }, w);
    if (vw < SMALL_VIEWPORT) return clampPos({ x: (vw - w) / 2, y: vh - 80 }, w); // mount 后按实测高度校正
    if (initial === 'center') {
      return clampPos({ x: (vw - w) / 2, y: Math.max(24, (vh - 200) / 2) }, w);
    }
    return clampPos({ x: initial.x, y: initial.y }, w);
  }

  /** clamp:水平至少留 40px 可见;垂直顶边不出视口、底边留 40px。 */
  function clampPos(p: Pos, w: number): Pos {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    return {
      x: Math.min(Math.max(p.x, MIN_VISIBLE - w), vw - MIN_VISIBLE),
      y: Math.min(Math.max(p.y, 0), vh - MIN_VISIBLE),
    };
  }

  const widthStyle = $derived(small ? `min(96vw, ${width}px)` : `${width}px`);

  // ---- 拖拽:Pointer Events + setPointerCapture(标题栏 touch-none) ----
  interface DragState {
    pointerId: number;
    startX: number;
    startY: number;
    originX: number;
    originY: number;
  }
  let drag: DragState | null = null;

  function onHeaderPointerDown(e: PointerEvent): void {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      originX: pos.x,
      originY: pos.y,
    };
  }

  function onHeaderPointerMove(e: PointerEvent): void {
    if (!drag || drag.pointerId !== e.pointerId) return;
    pos = clampPos(
      {
        x: drag.originX + e.clientX - drag.startX,
        y: drag.originY + e.clientY - drag.startY,
      },
      panelPxWidth(),
    );
  }

  function onHeaderPointerEnd(e: PointerEvent): void {
    if (!drag || drag.pointerId !== e.pointerId) return;
    drag = null;
    const el = e.currentTarget as HTMLElement;
    if (el.hasPointerCapture(e.pointerId)) el.releasePointerCapture(e.pointerId);
    writePersisted();
  }

  /** 点击任意处:置顶并记为 active。 */
  function bringToFront(): void {
    z = ++zTop;
    activePanel = panelId;
  }

  function toggleFold(): void {
    folded = !folded;
    writePersisted();
  }

  const ARROW_DELTAS: Record<string, Pos> = {
    ArrowLeft: { x: -KEY_STEP, y: 0 },
    ArrowRight: { x: KEY_STEP, y: 0 },
    ArrowUp: { x: 0, y: -KEY_STEP },
    ArrowDown: { x: 0, y: KEY_STEP },
  };

  /** 标题栏自身聚焦时:Enter 切换折叠,方向键移动 ±16px 并写回持久化。 */
  function onHeaderKeydown(e: KeyboardEvent): void {
    if (e.target !== e.currentTarget) return; // 焦点在内部按钮上时走按钮默认行为
    if (e.key === 'Enter') {
      e.preventDefault();
      toggleFold();
      return;
    }
    const delta = ARROW_DELTAS[e.key];
    if (!delta) return;
    e.preventDefault();
    pos = clampPos({ x: pos.x + delta.x, y: pos.y + delta.y }, panelPxWidth());
    writePersisted();
  }

  onMount(() => {
    small = window.innerWidth < SMALL_VIEWPORT;
    if (saved) return; // 持久化位置优先,不做居中校正
    const el = panelEl;
    if (!el) return;
    const w = panelPxWidth();
    const h = el.offsetHeight; // 实测高度替换估算,校正一次
    if (small) {
      pos = clampPos({ x: (window.innerWidth - w) / 2, y: window.innerHeight - h - 12 }, w);
    } else if (initial === 'center') {
      pos = clampPos({ x: (window.innerWidth - w) / 2, y: (window.innerHeight - h) / 2 }, w);
    }
  });
</script>

<div
  bind:this={panelEl}
  role="dialog"
  aria-modal="false"
  aria-label={title}
  tabindex="-1"
  data-testid={testid}
  class="fixed rounded-lg border border-line bg-[#101218]/95 shadow-xl backdrop-blur"
  style:width={widthStyle}
  style:left={`${pos.x}px`}
  style:top={`${pos.y}px`}
  style:z-index={`${z}`}
  onpointerdown={bringToFront}
>
  <!-- 标题栏 = 拖拽把手;role="toolbar" 让其可聚焦,方向键/Enter 可达 -->
  <div
    role="toolbar"
    aria-label={`${title} 标题栏`}
    tabindex="0"
    data-testid={tid('header')}
    class="flex cursor-move touch-none select-none items-center justify-between gap-2 rounded-t-lg px-3 py-1.5 text-xs text-slate-200"
    onpointerdown={onHeaderPointerDown}
    onpointermove={onHeaderPointerMove}
    onpointerup={onHeaderPointerEnd}
    onpointercancel={onHeaderPointerEnd}
    onkeydown={onHeaderKeydown}
  >
    <span class="max-w-[20ch] truncate" data-testid={tid('title')}>{title}</span>
    <div class="flex flex-none items-center gap-1">
      {#if headerActions}
        {@render headerActions()}
      {/if}
      <button
        type="button"
        class="rounded px-1 leading-none text-slate-400 hover:text-white"
        aria-label={folded ? '展开' : '折叠'}
        aria-expanded={!folded}
        data-testid={tid('fold')}
        onclick={toggleFold}
      >
        {folded ? '▲' : '▼'}
      </button>
      <button
        type="button"
        class="rounded px-1 leading-none text-slate-400 hover:text-white"
        aria-label="关闭"
        data-testid={tid('close')}
        onclick={() => onclose?.()}
      >
        ✕
      </button>
    </div>
  </div>

  {#if !folded}
    <div class="p-3 text-xs" data-testid={tid('body')}>
      {@render children()}
    </div>
  {/if}
</div>
