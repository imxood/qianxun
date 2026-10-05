<script lang="ts">
  import { call } from '../lib/ipc';
  import { contextMenu } from '../lib/menu.svelte';
  import { nav, type PageId } from '../stores/nav.svelte';
  import { toast } from '../stores/toast.svelte';

  /** 主导航项：settings 独立沉底（低频项与工具项分组）。 */
  const items: Array<{ id: PageId; label: string; icon: string; detachable: boolean }> = [
    {
      id: 'env',
      label: '环境',
      icon: 'M4 7h16M4 7a2 2 0 012-2h2m10 0a2 2 0 012 2M6 5v14m12-14v14M6 19h12',
      detachable: false,
    },
    {
      id: 'dsh',
      label: 'DSH',
      icon: 'M12 3l8 4.5v9L12 21l-8-4.5v-9zM12 12l8-4.5M12 12v9M12 12L4 7.5',
      detachable: true,
    },
    {
      id: 'files',
      label: '文件',
      icon: 'M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2z',
      detachable: false,
    },
    {
      id: 'remote',
      label: '远程',
      icon: 'M8 2h8a2 2 0 012 2v16a2 2 0 01-2 2H8a2 2 0 01-2-2V4a2 2 0 012-2zM10 18h4',
      detachable: false,
    },
    {
      id: 'notes',
      label: '笔记',
      icon: 'M7 3h10a2 2 0 012 2v14a2 2 0 01-2 2H7a2 2 0 01-2-2V5a2 2 0 012-2zM9 8h6M9 12h6M9 16h4',
      detachable: false,
    },
    {
      id: 'web',
      label: '联网',
      icon: 'M12 3a9 9 0 100 18 9 9 0 000-18zM3 12h18M12 3c2.5 2.6 3.8 5.7 3.8 9S14.5 18.4 12 21c-2.5-2.6-3.8-5.7-3.8-9S9.5 5.6 12 3z',
      detachable: false,
    },
    {
      id: 'intel',
      label: '情报',
      icon: 'M12 2a10 10 0 100 20 10 10 0 000-20zM16.5 7.5l-2.6 6.4-6.4 2.6 2.6-6.4z',
      detachable: false,
    },
    {
      id: 'games',
      label: '游戏',
      icon: 'M6 9h4m-2-2v4M15 8h.01M18 10h.01M7.5 4h9A4.5 4.5 0 0121 8.5v5a4.5 4.5 0 01-4.5 4.5h-9A4.5 4.5 0 013 13.5v-5A4.5 4.5 0 017.5 4zM8 14h2m-1-1v2m7-1h.01M17.5 15h.01',
      detachable: false,
    },
    {
      id: 'plugins',
      label: '插件',
      icon: 'M9 3v4M15 3v4M6 7h12v3a6 6 0 01-12 0V7zM12 16v5',
      detachable: false,
    },
  ];

  /** 可分离视图类型（与 Rust standalone_view_meta 对应）。 */
  const DETACHABLE_VIEWS = new Set<string>(['dsh']);

  /** 分离中（spawn 未返回）的视图，防止重复右键连点开两个窗。 */
  let detaching: PageId | null = null;
  /** 键盘导航的焦点下标（跟随 nav.page，落在 visibleItems 坐标系里）。 */
  let focusIndex = $state(0);

  /**
   * 分离到独立窗口：spawn 成功才让位（失败保持原状）。
   * 失败给用户可见反馈（分析 S6/P2：此前只有 console.error，零反馈）。
   */
  async function detachToWindow(item: { id: PageId; label: string }): Promise<void> {
    // in-flight 守卫：spawn 返回前不接受第二次分离，
    // 否则窗口标签数与 nav.detached 计数可能不同步。
    if (detaching === item.id) return;
    detaching = item.id;
    try {
      await call<string>('window_spawn_view', { view: item.id });
      nav.detach(item.id);
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      console.error(`分离「${item.label}」失败`, error);
      toast.show(`分离「${item.label}」失败：${reason}`);
    } finally {
      detaching = null;
    }
  }

  function navItemMenu(event: MouseEvent, item: { id: PageId; label: string }): void {
    if (!DETACHABLE_VIEWS.has(item.id)) return;
    contextMenu.show(event, [
      {
        label: `分离「${item.label}」到独立窗口`,
        onclick: () => void detachToWindow(item),
      },
    ]);
  }

  /** 侧栏上「当前真实存在」的入口（未分离的项 + settings），键盘导航的操作对象。 */
  const visibleItems = $derived([
    ...items.filter((item) => !nav.detached[item.id]),
    { id: 'settings' as PageId, label: '设置', icon: '', detachable: false },
  ]);

  /**
   * 键盘导航（docs/18 C4）：10 个入口此前只能用鼠标点。
   * - ↑/↓ 或 k/j：上下移动并**立即跳转**（跟随式，不需要二次回车）
   * - Home/End：首/末项
   * - 数字键 1-9：直接跳到第 n 项（10 个入口里最后一个是 settings，只用 1-9）
   * 焦点始终跟随跳转目标，屏幕阅读器能读到 aria-current。
   */
  function moveFocus(delta: number): void {
    const list = visibleItems;
    if (list.length === 0) return;
    const current = list.findIndex((item) => item.id === nav.page);
    // 当前页不在列表里（刚被分离）时，从头开始。
    const base = current === -1 ? (delta > 0 ? -1 : 0) : current;
    const next = (base + delta + list.length) % list.length;
    focusIndex = next;
    nav.go(list[next]!.id);
  }

  function focusEdge(edge: 'first' | 'last'): void {
    const list = visibleItems;
    if (list.length === 0) return;
    focusIndex = edge === 'first' ? 0 : list.length - 1;
    nav.go(list[focusIndex]!.id);
  }

  function jumpToNumber(digit: number): void {
    // 1-9 对应前 9 个工具项；settings 是第 10 个，不占数字键。
    const target = visibleItems[digit - 1];
    if (!target) return;
    focusIndex = digit - 1;
    nav.go(target.id);
  }

  function onNavKeydown(event: KeyboardEvent): void {
    // 输入框/编辑器里打字时不抢键。
    const target = event.target as HTMLElement | null;
    if (target) {
      const tag = target.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || target.isContentEditable) {
        return;
      }
    }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    switch (event.key) {
      case 'ArrowDown':
      case 'j':
        event.preventDefault();
        moveFocus(1);
        return;
      case 'ArrowUp':
      case 'k':
        event.preventDefault();
        moveFocus(-1);
        return;
      case 'Home':
        event.preventDefault();
        focusEdge('first');
        return;
      case 'End':
        event.preventDefault();
        focusEdge('last');
        return;
      default:
        break;
    }
    if (/^[1-9]$/.test(event.key)) {
      event.preventDefault();
      jumpToNumber(Number(event.key));
    }
  }

  /** 让当前项的按钮拿到焦点（与 visibleItems 同步，避免分离后焦点悬空）。 */
  $effect(() => {
    const list = visibleItems;
    if (focusIndex < 0 || focusIndex >= list.length) focusIndex = Math.max(0, list.length - 1);
  });
</script>

<!--
  纯图标窄栏（48px）：位置感由左缘指示条 + 图标同色承担，文字收敛为
  tooltip / aria-label。比文字纵排窄 16px，视觉也更静。
-->
<!--
  svelte-ignore a11y_no_noninteractive_element_interactions
  键盘导航（docs/18 C4）挂在 nav 上：10 个入口此前只能用鼠标点。
  `<nav>` 是导航 landmark，给它绑 keydown 属于 roving-tabindex 的标准做法
  （方向键在按钮组间移动、数字键直达），告警规则不区分这一类。
  实际焦点通过每个按钮的 tabindex=0/-1 落在真按钮上，键盘用户拿到的仍是
  按钮语义与 aria-current。
-->
<nav
  class="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-line bg-surface py-2"
  aria-label="主导航"
  onkeydown={onNavKeydown}
>
  {#each items.filter((item) => !nav.detached[item.id]) as item, index (item.id)}
    {@const active = nav.page === item.id}
    <button
      class="group relative flex size-9 items-center justify-center rounded-lg outline-none transition-colors focus-visible:ring-2 focus-visible:ring-accent/50 {active
        ? 'qx-nav-active'
        : 'text-muted hover:bg-accent-soft/50 hover:text-fg'}"
      aria-label={item.label}
      aria-current={active ? 'page' : undefined}
      tabindex={index === focusIndex ? 0 : -1}
      data-testid="nav-{item.id}"
      onclick={() => {
        focusIndex = index;
        nav.go(item.id);
      }}
      oncontextmenu={(event) => navItemMenu(event, item)}
      title={item.detachable ? `${item.label} · 右键分离到独立窗口` : item.label}
    >
      <svg
        viewBox="0 0 24 24"
        class="size-[18px] shrink-0 transition-colors"
        fill="none"
        stroke="currentColor"
        stroke-width={active ? 1.9 : 1.6}
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d={item.icon} />
      </svg>
    </button>
  {/each}

  {#if !nav.detached.settings}
    <!-- 设置沉底：与工具项分组，栏内层级一眼可辨。 -->
    {@const settingsIndex = visibleItems.length - 1}
    <div class="mt-auto flex w-full justify-center border-t border-line/60 pt-1.5">
      <button
        class="group relative flex size-9 items-center justify-center rounded-lg outline-none transition-colors focus-visible:ring-2 focus-visible:ring-accent/50 {nav.page ===
        'settings'
          ? 'qx-nav-active'
          : 'text-muted hover:bg-accent-soft/50 hover:text-fg'}"
        aria-label="设置"
        aria-current={nav.page === 'settings' ? 'page' : undefined}
        tabindex={settingsIndex === focusIndex ? 0 : -1}
        data-testid="nav-settings"
        onclick={() => {
          focusIndex = settingsIndex;
          nav.go('settings');
        }}
        title="设置"
      >
        <svg
          viewBox="0 0 24 24"
          class="size-[18px] shrink-0 transition-colors"
          fill="none"
          stroke="currentColor"
          stroke-width={nav.page === 'settings' ? 1.9 : 1.6}
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path
            d="M12 8a4 4 0 100 8 4 4 0 000-8zM19 12l2-1-2-4-2 1-2-1V4h-2.2L12 6l-2.8-2H7v3l-2 1-2-1-2 4 2 1v2l-2 1 2 4 2-1 2 1v3h2.8L12 18l2.8 2H17v-3l2-1 2 1 2-4-2-1z"
          />
        </svg>
      </button>
    </div>
  {/if}
</nav>
