<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { contextMenu, type MenuItem } from '../../lib/menu.svelte';
  import {
    fileIconClass,
    FILE_ICON_PATH,
    fileKind,
    highlightName,
    splitHighlightedPath,
    type FileKind,
  } from './fileIcon';
  import { formatBytes, formatTime } from './format';
  import { absolutePath, copyMenuItems, copyText, locateInExplorer, openFile } from './locate';
  import RootBar from './RootBar.svelte';
  import type { FileHit } from '../../lib/ipc/contract';

  // ---- 排序 / 类型过滤 ------------------------------------------------
  type SortKey = 'score' | 'name' | 'mtime' | 'size';
  const KIND_ORDER: FileKind[] = ['code', 'doc', 'image', 'archive', 'other'];
  let sortKey = $state<SortKey>('score');
  let sortAsc = $state(false);
  let kindFilter = $state<FileKind | 'all'>('all');

  function setSort(key: SortKey): void {
    if (sortKey === key) {
      sortAsc = !sortAsc;
      return;
    }
    sortKey = key;
    // 名称升序最自然；分数/大小/时间「大/新在前」。
    sortAsc = key === 'name';
  }

  const ARROW = { asc: '↑', desc: '↓' } as const;

  const kindCounts = $derived.by(() => {
    const counts: Partial<Record<FileKind, number>> = {};
    for (const hit of search.filesResult?.items ?? []) {
      const kind = fileKind(hit.path);
      counts[kind] = (counts[kind] ?? 0) + 1;
    }
    return counts;
  });

  const filtered = $derived.by(() => {
    const all = search.filesResult?.items ?? [];
    return kindFilter === 'all' ? all : all.filter((hit) => fileKind(hit.path) === kindFilter);
  });

  const sorted = $derived.by(() => {
    const list = [...filtered];
    list.sort((a, b) => {
      const va = sortValue(a);
      const vb = sortValue(b);
      const cmp = va < vb ? -1 : va > vb ? 1 : 0;
      return sortAsc ? cmp : -cmp;
    });
    return list;
  });

  function sortValue(hit: FileHit): string | number {
    switch (sortKey) {
      case 'name':
        return hit.path.toLowerCase();
      case 'mtime':
        return hit.mtime;
      case 'size':
        return hit.size;
      default:
        return hit.score;
    }
  }

  // ---- 多选（Ctrl 点选 / Shift 范围 / 右键整组操作）--------------------
  let selected: string[] = $state([]);
  let anchor = $state(-1);
  let cursor = $state(-1);

  function selectRow(event: MouseEvent, hit: FileHit, index: number): void {
    if (event.ctrlKey || event.metaKey) {
      toggleSelect(hit, index, true);
    } else if (event.shiftKey) {
      rangeSelect(index);
    } else {
      clearSelection();
      anchor = index;
      cursor = index;
    }
  }

  function toggleSelect(hit: FileHit, index: number, additive: boolean): void {
    selected = additive
      ? selected.includes(hit.path)
        ? selected.filter((path) => path !== hit.path)
        : [...selected, hit.path]
      : selected.includes(hit.path)
        ? selected
        : [...selected, hit.path];
    anchor = index;
    cursor = index;
  }

  function rangeSelect(index: number): void {
    const [from, to] =
      anchor >= 0
        ? [Math.min(anchor, index), Math.max(anchor, index)]
        : [
            cursor >= 0 ? Math.min(cursor, index) : index,
            cursor >= 0 ? Math.max(cursor, index) : index,
          ];
    selected = sorted.slice(from, to + 1).map((hit) => hit.path);
    cursor = index;
  }

  function clearSelection(): void {
    selected = [];
    anchor = -1;
    cursor = -1;
  }

  function rowMenu(event: MouseEvent, hit: FileHit, index: number): void {
    if (!selected.includes(hit.path)) {
      selected = [hit.path];
      anchor = index;
      cursor = index;
    }
    contextMenu.show(event, menuFor(selected));
  }

  /** 右键菜单：单选给打开/定位/三种路径形态；多选给批量复制。 */
  function menuFor(paths: string[]): MenuItem[] {
    const items: MenuItem[] = [];
    const only = paths.length === 1 ? paths[0] : undefined;
    if (only) {
      items.push({ label: '打开', onclick: () => openFile(only) });
      items.push({ label: '打开所在位置', onclick: () => locateInExplorer(only) });
      items.push(...copyMenuItems(only));
    } else {
      const joined = paths.map((path) => absolutePath(path) ?? path).join('\r\n');
      items.push({
        label: `打开所在位置（${paths.length} 个）`,
        onclick: () => paths.forEach((path) => locateInExplorer(path)),
      });
      items.push({ label: `复制 ${paths.length} 个路径`, onclick: () => void copyText(joined) });
    }
    return items;
  }

  // ---- 键盘流（查询框内：↑↓ 移光标，Enter 开光标行，Ctrl+Shift+C 复制）--
  function onListKey(event: KeyboardEvent): void {
    if (sorted.length === 0) return;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const delta = event.key === 'ArrowDown' ? 1 : -1;
      cursor = Math.min(sorted.length - 1, Math.max(0, cursor + delta));
      const hit = sorted[cursor];
      if (hit) {
        selected = [hit.path];
        anchor = cursor;
      }
    } else if (event.key === 'Enter') {
      const hit = sorted[cursor];
      if (hit) openFile(hit.path);
    } else if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 'c') {
      event.preventDefault();
      const source = selected.length > 0 ? selected : [sorted[cursor]?.path ?? ''];
      const paths = source
        .filter((path) => path.length > 0)
        .map((path) => absolutePath(path) ?? path);
      if (paths.length > 0) void copyText(paths.join('\r\n'));
    }
  }
</script>

<section class="mx-auto w-full max-w-5xl space-y-4">
  <RootBar />

  <div class="flex items-center gap-3">
    <div class="relative min-w-0 flex-1">
      <svg
        viewBox="0 0 24 24"
        class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        aria-hidden="true"
      >
        <path
          d="M14 4a6.5 6.5 0 10-4.3 11.4L15 21M14 4l-4.3 11.4M14 4c3 1 4.6 3.6 4.6 6.5S17 16 14 17"
        />
      </svg>
      <input
        class="w-full rounded-md border border-line bg-surface py-2 pl-9 pr-3 text-sm placeholder:text-muted/60"
        type="text"
        placeholder={search.status?.root
          ? '按文件名过滤，即时生效（↑↓ 选择 · Enter 打开）'
          : '先选择目录'}
        disabled={!search.status?.root}
        bind:value={search.filesQuery}
        oninput={() => search.scheduleFiles()}
        onkeydown={(event) => {
          if (event.key === 'Escape') {
            search.filesQuery = '';
            search.filesResult = null;
            clearSelection();
          } else {
            onListKey(event);
          }
        }}
      />
    </div>
    {#if search.filesBusy}
      <span
        class="inline-block size-4 shrink-0 animate-spin rounded-full border-2 border-line border-t-accent"
        aria-hidden="true"
      ></span>
    {:else if search.filesResult}
      <span class="shrink-0 text-xs text-muted">{filtered.length} 个结果</span>
    {/if}
  </div>

  {#if filtered.length > 0}
    <div class="flex flex-wrap items-center gap-1.5 text-xs">
      <button
        class="rounded-full px-2 py-0.5 transition-colors {kindFilter === 'all'
          ? 'bg-accent text-white'
          : 'bg-surface text-muted hover:text-fg'}"
        onclick={() => (kindFilter = 'all')}
      >
        全部 {search.filesResult?.items.length ?? 0}
      </button>
      {#each KIND_ORDER as kind (kind)}
        {#if (kindCounts[kind] ?? 0) > 0}
          <button
            class="rounded-full px-2 py-0.5 transition-colors {kindFilter === kind
              ? 'bg-accent text-white'
              : 'bg-surface text-muted hover:text-fg'}"
            onclick={() => (kindFilter = kind)}
          >
            {kind}{kindCounts[kind]}
          </button>
        {/if}
      {/each}
    </div>

    <div class="overflow-hidden rounded-lg border border-line bg-card">
      <div
        class="flex items-center border-b border-line/60 bg-surface/40 px-3 py-1.5 text-xs text-muted"
      >
        <button class="min-w-0 flex-1 text-left hover:text-fg" onclick={() => setSort('name')}>
          名称 {sortKey === 'name' ? ARROW[sortAsc ? 'asc' : 'desc'] : ''}
        </button>
        <button class="w-20 text-right hover:text-fg" onclick={() => setSort('size')}>
          大小 {sortKey === 'size' ? ARROW[sortAsc ? 'asc' : 'desc'] : ''}
        </button>
        <button class="w-36 text-right hover:text-fg" onclick={() => setSort('mtime')}>
          修改时间 {sortKey === 'mtime' ? ARROW[sortAsc ? 'asc' : 'desc'] : ''}
        </button>
      </div>
      <div class="divide-y divide-line/40">
        {#each sorted as hit, index (hit.path)}
          {@const { directory, name, nameOffsets } = splitHighlightedPath(hit.path, hit.offsets)}
          <div
            class="flex h-8 cursor-default select-none items-center px-3 transition-colors {selected.includes(
              hit.path,
            )
              ? 'bg-accent-soft'
              : index === cursor
                ? 'bg-accent-soft/50'
                : 'hover:bg-accent-soft/40'}"
            title={hit.path}
            role="row"
            onclick={(event) => selectRow(event, hit, index)}
            ondblclick={() => openFile(hit.path)}
            oncontextmenu={(event) => rowMenu(event, hit, index)}
          >
            <span class="flex min-w-0 flex-1 items-center gap-2 font-mono text-sm">
              <svg
                viewBox="0 0 24 24"
                class="size-4 shrink-0 {fileIconClass(hit.path)}"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d={FILE_ICON_PATH} />
              </svg>
              <span class="min-w-0 truncate">
                <span class="text-muted/70">{directory}</span
                >{#each highlightName(name, nameOffsets) as segment, i (hit.path + i)}{#if segment.matched}<mark
                      class="rounded bg-accent-soft text-fg">{segment.text}</mark
                    >{:else}{segment.text}{/if}{/each}
              </span>
            </span>
            <span class="w-20 shrink-0 text-right text-xs text-muted tabular-nums">
              {formatBytes(hit.size)}
            </span>
            <span class="w-36 shrink-0 text-right text-xs text-muted tabular-nums">
              {formatTime(hit.mtime)}
            </span>
          </div>
        {/each}
      </div>
    </div>
    <p class="text-xs text-muted/70">
      单击选中 · Ctrl 单击加选 · Shift 单击范围 · 双击打开 · 右键更多操作
    </p>
  {:else if search.filesResult}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-fg">无匹配文件</p>
      <p class="text-xs text-muted">换个关键词试试</p>
    </div>
  {:else if search.status?.root}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-fg">输入文件名开始查找</p>
      <p class="text-xs text-muted">支持模糊匹配，结果按相关度排序</p>
    </div>
  {/if}
</section>
