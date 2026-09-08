<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { revealItemInDir } from '@tauri-apps/plugin-opener';
  import { call } from '../../../lib/ipc';
  import { contextMenu } from '../../../lib/menu.svelte';
  import type { DiskEntry, DiskHome } from '../../../lib/ipc/contract';
  import { formatBytes } from '../format';
  import { squarify, type Rect } from './treemap';

  // ---- 状态 --------------------------------------------------------------
  let home = $state<DiskHome | null>(null);
  /** 当前扫描结果（方块 = 它的子项）。 */
  let current = $state<DiskEntry | null>(null);
  /** 面包屑扫描链：trail 末项即 current。 */
  let trail = $state<DiskEntry[]>([]);
  let scanning = $state(false);
  let view = $state<'blocks' | 'list'>('blocks');
  let actionError = $state('');
  let cleanTarget = $state<DiskEntry | null>(null);
  let cleaning = $state(false);
  /** 用户添加过的外部目录（本地记忆，最多 6 个）。 */
  let externals = $state<string[]>(loadExternals());

  const EXTERNALS_KEY = 'qx-disk-externals';

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

  // ---- 扫描（代际防竞态）--------------------------------------------------
  let seq = 0;
  type ScanMode = 'reset' | 'push' | 'refresh';

  async function scan(path: string, mode: ScanMode): Promise<void> {
    const ticket = ++seq;
    scanning = true;
    actionError = '';
    try {
      const entry = await call<DiskEntry>('disk_scan', { path });
      if (ticket !== seq) return;
      current = entry;
      trail =
        mode === 'reset'
          ? [entry]
          : mode === 'push'
            ? [...trail, entry]
            : [...trail.slice(0, -1), entry];
    } catch (error) {
      if (ticket === seq) actionError = error instanceof Error ? error.message : String(error);
    } finally {
      if (ticket === seq) scanning = false;
    }
  }

  onMount(() => {
    void (async () => {
      try {
        home = await call<DiskHome>('disk_home');
        await scan(home.root, 'reset');
      } catch (error) {
        actionError = error instanceof Error ? error.message : String(error);
      }
    })();
  });

  function drill(entry: DiskEntry): void {
    if (!entry.dir || !entry.path) return;
    void scan(entry.path, 'push');
  }

  function crumbTo(index: number): void {
    if (index === trail.length - 1) return;
    const target = trail[index];
    if (!target) return;
    trail = trail.slice(0, index);
    void scan(target.path, 'push');
  }

  function goHome(): void {
    if (home) void scan(home.root, 'reset');
  }

  async function pickExternal(): Promise<void> {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== 'string' || !selected.trim()) return;
    externals = [selected, ...externals.filter((item) => item !== selected)].slice(0, 6);
    saveExternals();
    await scan(selected, 'reset');
  }

  function removeExternal(path: string, event: MouseEvent): void {
    event.stopPropagation();
    externals = externals.filter((item) => item !== path);
    saveExternals();
  }

  // ---- 清理（确认后走回收站）----------------------------------------------
  function askClean(entry: DiskEntry): void {
    if (!entry.path) return;
    cleanTarget = entry;
  }

  async function confirmClean(): Promise<void> {
    if (!cleanTarget || !current) return;
    cleaning = true;
    actionError = '';
    const refreshPath = current.path;
    try {
      await call('disk_clean', { path: cleanTarget.path });
      cleanTarget = null;
      if (refreshPath) await scan(refreshPath, 'refresh');
    } catch (error) {
      actionError = error instanceof Error ? error.message : String(error);
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

  // ---- treemap 布局 ---------------------------------------------------------
  let box = $state<HTMLDivElement | null>(null);
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
    if (!current || view !== 'blocks' || boxSize.w <= 0 || boxSize.h <= 0) return [];
    const scanned = current;
    const total = scanned.size;
    const rects = squarify(
      scanned.children.map((entry) => Math.max(entry.size, 1)),
      { x: 0, y: 0, w: boxSize.w, h: boxSize.h },
    );
    return scanned.children.map((entry, index) => {
      const share = total > 0 ? entry.size / total : 0;
      const mix = entry.dir ? Math.round(10 + 55 * Math.sqrt(share)) : 0;
      return { entry, rect: rects[index]!, mix };
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

  const crumbs = $derived(
    trail.map((entry, index) => ({
      entry,
      index,
      label:
        index === 0 && home && entry.path === home.root
          ? '数据目录'
          : (home?.labels[entry.name] ?? entry.name),
    })),
  );

  function pathTail(path: string): string {
    const trimmed = path.replace(/[\\/]+$/, '');
    return trimmed.split(/[\\/]/).pop() ?? trimmed;
  }
</script>

<section class="flex h-full flex-col gap-3">
  <!-- 路径行：面包屑 + 汇总 + 视图切换 + 动作 -->
  <div class="flex shrink-0 items-center gap-2">
    <nav class="flex min-w-0 flex-1 items-center gap-1 text-sm" aria-label="目录层级">
      {#each crumbs as crumb (crumb.entry.path)}
        {#if crumb.index > 0}
          <span class="shrink-0 text-muted/60">/</span>
        {/if}
        <button
          class="max-w-44 truncate rounded px-1.5 py-0.5 transition-colors {crumb.index ===
          crumbs.length - 1
            ? 'font-medium text-fg'
            : 'text-muted hover:bg-accent-soft hover:text-fg'}"
          title={crumb.entry.path}
          onclick={() => crumbTo(crumb.index)}
        >
          {crumb.label}
        </button>
      {/each}
      {#if current}
        <span class="ml-2 shrink-0 text-xs text-muted">
          {formatBytes(current.size)} · {current.children.length} 项
        </span>
      {/if}
    </nav>

    <div class="flex shrink-0 items-center gap-1 rounded-md border border-line bg-surface p-0.5">
      <button
        class="rounded px-2 py-1 text-xs transition-colors {view === 'blocks'
          ? 'bg-accent-soft font-medium text-fg'
          : 'text-muted hover:text-fg'}"
        onclick={() => (view = 'blocks')}
      >
        方块
      </button>
      <button
        class="rounded px-2 py-1 text-xs transition-colors {view === 'list'
          ? 'bg-accent-soft font-medium text-fg'
          : 'text-muted hover:text-fg'}"
        onclick={() => (view = 'list')}
      >
        列表
      </button>
    </div>
    <button
      class="shrink-0 rounded-md border border-line px-2.5 py-1.5 text-xs text-fg transition-colors hover:bg-accent-soft"
      onclick={() => current && void scan(current.path, 'refresh')}
    >
      重新扫描
    </button>
    <button
      class="shrink-0 rounded-md bg-accent px-2.5 py-1.5 text-xs font-medium text-white transition-colors hover:bg-accent/90"
      onclick={() => void pickExternal()}
    >
      添加目录
    </button>
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
            onclick={() => void scan(external, 'reset')}
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

  <!-- 主体 -->
  <div class="relative min-h-0 flex-1 overflow-hidden rounded-lg border border-line bg-bg">
    {#if !current}
      <p class="flex h-full items-center justify-center text-sm text-muted">准备扫描…</p>
    {:else if scanning && current.children.length === 0}
      <p class="flex h-full items-center justify-center text-sm text-muted">扫描中…</p>
    {:else if current.children.length === 0}
      <p class="flex h-full items-center justify-center text-sm text-muted">空目录</p>
    {:else if view === 'blocks'}
      <div
        bind:this={box}
        class="absolute inset-0 transition-opacity {scanning ? 'opacity-40' : ''}"
        data-testid="disk-treemap"
      >
        {#each blocks as block (block.entry.path || block.entry.name)}
          <button
            class="group absolute overflow-hidden rounded-[3px] text-left transition-[filter] {block
              .entry.path
              ? 'hover:z-10 hover:ring-2 hover:ring-accent hover:brightness-105'
              : 'border border-dashed border-line'}"
            style={blockStyle(block)}
            title="{block.entry.path || block.entry.name} · {formatBytes(block.entry.size)}"
            aria-label="{block.entry.name}，占用 {formatBytes(block.entry.size)}"
            onclick={() => drill(block.entry)}
            oncontextmenu={(event) => menuFor(event, block.entry)}
          >
            {#if showLabel(block.rect) && block.entry.path}
              <span
                class="pointer-events-none absolute inset-0 flex flex-col justify-between p-1.5"
              >
                <span class="truncate text-xs font-medium leading-4 text-fg">
                  {home?.labels[block.entry.name] ?? block.entry.name}
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
        class="absolute inset-0 overflow-y-auto transition-opacity {scanning ? 'opacity-40' : ''}"
        data-testid="disk-list"
      >
        <ul class="divide-y divide-line">
          {#each current.children as entry (entry.path || entry.name)}
            <button
              class="flex w-full items-center gap-3 px-4 py-2 text-left text-sm transition-colors hover:bg-accent-soft"
              onclick={() => drill(entry)}
              oncontextmenu={(event) => menuFor(event, entry)}
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
                  <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
                {:else}
                  <path d="M7 3h7l4 4v14H7zM14 3v4h4" />
                {/if}
              </svg>
              <span class="min-w-0 flex-1">
                <span class="block truncate text-fg">
                  {home?.labels[entry.name] ?? entry.name}
                </span>
                <span class="mt-1 block h-1 w-full overflow-hidden rounded-full bg-line">
                  <span
                    class="block h-full rounded-full bg-accent/70"
                    style={`width:${current.size > 0 ? Math.max(1, (entry.size / current.size) * 100) : 0}%`}
                  ></span>
                </span>
              </span>
              <span class="w-20 shrink-0 text-right font-mono text-xs text-muted">
                {formatBytes(entry.size)}
              </span>
            </button>
          {/each}
        </ul>
      </div>
    {/if}

    {#if scanning && current && current.children.length > 0}
      <span
        class="absolute right-3 top-3 flex items-center gap-1.5 rounded-full bg-card px-2.5 py-1 text-xs text-accent shadow"
        role="status"
      >
        <span
          class="inline-block size-3 animate-spin rounded-full border-2 border-line border-t-accent"
          aria-hidden="true"
        ></span>
        扫描中…
      </span>
    {/if}
  </div>

  {#if home}
    <button
      class="shrink-0 self-start text-xs text-muted transition-colors hover:text-fg"
      onclick={goHome}
    >
      返回数据目录
    </button>
  {/if}
</section>

<!-- 清理确认：回收站可还原，但依然确认后执行 -->
{#if cleanTarget}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6">
    <div
      class="w-full max-w-md rounded-lg border border-line bg-card p-5 shadow-xl"
      role="alertdialog"
      aria-modal="true"
      aria-label="确认移入回收站"
      data-testid="disk-clean-confirm"
    >
      <h2 class="text-sm font-medium">移入回收站</h2>
      <p class="mt-3 break-all rounded-md bg-bg p-2 font-mono text-xs text-muted">
        {cleanTarget.path}
      </p>
      <p class="mt-3 text-sm text-fg">
        占用 {formatBytes(cleanTarget.size)}。移入回收站，可在回收站还原。
      </p>
      <div class="mt-5 flex justify-end gap-2">
        <button
          class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft"
          onclick={() => (cleanTarget = null)}
        >
          取消
        </button>
        <button
          class="rounded-md bg-danger px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-danger/90 disabled:opacity-50"
          disabled={cleaning}
          data-testid="disk-clean-confirm-button"
          onclick={() => void confirmClean()}
        >
          {cleaning ? '清理中…' : '移入回收站'}
        </button>
      </div>
    </div>
  </div>
{/if}
