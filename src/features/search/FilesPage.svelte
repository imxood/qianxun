<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { showHitContextMenu } from './components/hit-context-menu';
  import {
    fileIconClass,
    FILE_ICON_PATH,
    fileKind,
    highlightName,
    splitHighlightedPath,
    type FileKind,
  } from './fileIcon';
  import { formatBytes, formatTime } from './format';
  import { absolutePath, copyText, openFile } from './locate';
  import RootBar from './RootBar.svelte';
  import type { FileHit, SearchFilesMode } from '../../lib/ipc/contract';

  // ---- 严格度切换（汇总 P1）---------------------------------------------
  const MODES: Array<{ id: SearchFilesMode; label: string; hint: string }> = [
    { id: 'fuzzy', label: 'Fuzzy', hint: '模糊匹配：字符序列 + bigram 打分（默认）' },
    { id: 'substring', label: '包含', hint: '纯子串匹配（大小写不敏感）' },
    { id: 'regex', label: '正则', hint: '把整段输入当作正则表达式（大小写不敏感）' },
  ];

  /**
   * 续页页大小（汇总 P3）；和后端 `clamp(1, 500)` 上限对齐。store.searchMoreFiles
   * 已硬编码 200；此处保留常量作为 UI 文案/逻辑可调的参考点。
   */
  const PAGE_SIZE = 200;
  void PAGE_SIZE;

  // ---- 排序 / 类型过滤 ------------------------------------------------
  type SortKey = 'score' | 'name' | 'mtime' | 'size';
  const KIND_ORDER: FileKind[] = ['code', 'doc', 'image', 'archive', 'other'];
  const KIND_LABEL: Record<FileKind, string> = {
    code: '代码',
    doc: '文档',
    image: '图片',
    archive: '压缩包',
    other: '其他',
  };
  let sortKey = $state<SortKey>('score');
  let sortAsc = $state(false);
  let kindFilter = $state<FileKind | 'all'>('all');
  /** 续页累积（汇总 P3）：默认 sort=score 时可逐页追加；切 sort/filter 自动清空。 */
  let accumulated = $state<FileHit[]>([]);
  let accumulating = $state(false);
  /** 当前已加载到第几页（0=未加载，1=已加载 1 页即 ≤200 条）；totalMatched 决定能加载多少页。 */
  let loadedPages = $state(0);

  function setSort(key: SortKey): void {
    if (sortKey === key) {
      sortAsc = !sortAsc;
      return;
    }
    sortKey = key;
    // 名称升序最自然；分数/大小/时间「大/新在前」。
    sortAsc = key === 'name';
    // 切排序时续页累积不再稳定（需要全量重排），自动重置。
    accumulated = [];
    loadedPages = 0;
  }

  /** 续页：取下一页 append 到末尾。仅当 sort=score 时启用（见 canLoadMore）。 */
  async function loadMore(): Promise<void> {
    if (!canLoadMore || accumulating) return;
    const result = search.filesResult;
    if (!result) return;
    accumulating = true;
    try {
      const nextPage = await search.searchMoreFiles(
        search.filesQuery,
        result.totalMatched,
        accumulated.length,
      );
      // 后端按 score 排序追加到末尾是稳定的；新条目的 score 必须 ≤ 旧条目。
      accumulated = [...accumulated, ...nextPage];
      loadedPages += 1;
    } finally {
      accumulating = false;
    }
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

  /** fuzzy 模式弱匹配阈值：归一化到当前列表最高分的 < 50% 即视为弱（汇总 P2）。
   *  substring/regex 模式 score=1000 是人为给定的"匹配"标记，应永远视为强匹配。 */
  function isWeakMatch(hit: FileHit): boolean {
    if (search.filesMode !== 'fuzzy') return false;
    const items = search.filesResult?.items ?? [];
    if (items.length === 0) return false;
    const maxScore = items.reduce((m, h) => (h.score > m ? h.score : m), -Infinity);
    if (maxScore <= 0) return false;
    return hit.score < maxScore * 0.5;
  }

  /** 续页判定：仅当 sort=score 时续页才有意义（其他排序需全量重排）。 */
  const canLoadMore = $derived(
    sortKey === 'score' && accumulated.length < (search.filesResult?.totalMatched ?? 0),
  );

  /**
   * 同步 store → accumulated：当 query / mode 改变后 store 返回新第一页
   * 时，覆写 accumulated（不计累计）；切到 score 排序时由 setSort 主动清，
   * 此 effect 在 score 排序下承担"接住新一页"的作用。
   */
  $effect(() => {
    const result = search.filesResult;
    if (!result) {
      accumulated = [];
      loadedPages = 0;
      return;
    }
    // 当 accumulated 还没积累（仅第一页）时，跟着 store 同步；
    // 用户按了"加载更多"后，accumulated 会比 store.items 长，
    // 此时 store 是首页的"快照"，不覆盖我们的累积。
    if (accumulated.length === 0 || (loadedPages === 1 && result.items.length > 0)) {
      accumulated = [...result.items];
      loadedPages = result.items.length > 0 ? 1 : 0;
    }
  });

  const filtered = $derived.by(() => {
    // 数据源：accumulated（汇总 P3 续页累积）优先级高于 store 单页。
    // - query 改变 → runFiles 完成后 FilesPage 同步重置 accumulated
    // - sort 改变 → setSort 已重置 accumulated
    // - 续页 → loadMore 追加到末尾
    const all = accumulated.length > 0 ? accumulated : (search.filesResult?.items ?? []);
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
    // 统一走 components/hit-context-menu（汇总 §3.3 抽公共组件）。
    showHitContextMenu(event, hit.path, selected);
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
      // 汇总 §3.11 修：键盘移动光标时滚屏（汇总 02 §3.11 提的旧 bug）。
      // data-cursor-index 与 tabindex 配对；用 setTimeout 让响应式更新 commit 完再 scroll。
      queueMicrotask(() => {
        const row = document.querySelector<HTMLDivElement>(`[data-cursor-index="${cursor}"]`);
        row?.scrollIntoView({ block: 'nearest' });
      });
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
        class="qx-input w-full py-2 pl-9"
        type="text"
        placeholder={search.status?.root ? '按文件名过滤（↑↓ 选择 · Enter 打开）' : '先选择目录'}
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
    <!-- 严格度切换（汇总 P1）：fuzzy 容错 / substring 精确包含 / regex 模式匹配。
         切换立即重查；空 query 时禁用避免误发请求。 -->
    <div
      class="flex shrink-0 items-center gap-0.5 rounded-lg border border-line bg-surface p-1"
      role="tablist"
      aria-label="文件名搜索严格度"
    >
      {#each MODES as mode (mode.id)}
        <button
          role="tab"
          aria-selected={search.filesMode === mode.id}
          title={mode.hint}
          class="qx-segment {search.filesMode === mode.id ? 'qx-segment-on' : 'qx-segment-off'}"
          data-testid="files-mode-{mode.id}"
          onclick={() => {
            if (search.filesMode === mode.id) return;
            search.filesMode = mode.id;
            if (import.meta.env.DEV) {
              console.debug('[FilesPage] filesMode 切换为', mode.id, '→', search.filesMode);
            }
            if (search.filesQuery.trim()) search.scheduleFiles();
          }}
        >
          {mode.label}
        </button>
      {/each}
      <!-- 调试：当前 mode 显示（汇总 P1 严格度切换是否生效的肉眼证据）。 -->
      <span class="ml-1 text-xs text-muted" data-testid="files-mode-debug">
        当前：{search.filesMode}
      </span>
    </div>
    {#if search.filesBusy}
      <span
        class="inline-block size-4 shrink-0 animate-spin rounded-full border-2 border-line border-t-accent"
        aria-hidden="true"
      ></span>
    {:else if search.filesResult}
      <span class="shrink-0 text-xs text-muted" aria-live="polite">
        {#if search.filesResult.totalMatched > search.filesResult.items.length}
          前 {search.filesResult.items.length} 条 · 共 {search.filesResult.totalMatched} 条
        {:else if search.filesResult.totalMatched > 0}
          {search.filesResult.totalMatched} 个结果
        {:else if search.filesQuery.trim()}
          （无匹配）
        {/if}
      </span>
    {/if}
  </div>

  {#if filtered.length > 0}
    <div class="flex flex-wrap items-center gap-1.5 text-xs">
      <button
        class="qx-chip {kindFilter === 'all' ? 'qx-chip-on' : 'qx-chip-off'}"
        onclick={() => (kindFilter = 'all')}
      >
        全部 {search.filesResult?.items.length ?? 0}
      </button>
      {#each KIND_ORDER as kind (kind)}
        {#if (kindCounts[kind] ?? 0) > 0}
          <button
            class="qx-chip {kindFilter === kind ? 'qx-chip-on' : 'qx-chip-off'}"
            onclick={() => (kindFilter = kind)}
          >
            {KIND_LABEL[kind]}
            {kindCounts[kind]}
          </button>
        {/if}
      {/each}
    </div>

    <div class="qx-card overflow-hidden">
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
          {@const weak = isWeakMatch(hit)}
          <div
            class="flex h-8 cursor-pointer select-none items-center px-3 transition-colors focus-visible:bg-accent-soft/60 focus-visible:outline-none {selected.includes(
              hit.path,
            )
              ? 'bg-accent-soft'
              : index === cursor
                ? 'bg-accent-soft/50'
                : 'hover:bg-accent-soft/40'} {weak ? 'opacity-60' : ''}"
            title={weak ? `${hit.path}\n弱匹配：${hit.score} 分` : hit.path}
            role="row"
            tabindex={index === cursor ? 0 : -1}
            data-cursor-index={index}
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
                {#if weak}<span class="ml-1 text-warning" title="弱匹配：可能不是您要找的">·</span
                  >{/if}
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
      <!-- 续页按钮（汇总 P3）：仅在 score 排序且未达 totalMatched 时显示。
           数据增长方向：已加载 N / 共 M；点一下加 PAGE_SIZE。 -->
      {#if canLoadMore}
        <div class="flex justify-center pt-3">
          <button
            class="qx-btn qx-btn-outline qx-btn-sm"
            disabled={accumulating}
            onclick={() => void loadMore()}
          >
            {accumulating
              ? '加载中…'
              : `加载更多（已显示 ${accumulated.length} / ${search.filesResult?.totalMatched ?? 0}）`}
          </button>
        </div>
      {/if}
    </div>
  {:else if search.filesResult}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-fg">无匹配文件</p>
      <p class="text-xs text-muted">换个关键词试试</p>
    </div>
  {:else if search.filesError}
    <!-- 错误态：之前 catch 被吞，UI 永远"无结果"；汇总 §3.9 提议统一 lastError。 -->
    <div class="flex flex-col items-center gap-2 py-16 text-center" role="alert" aria-live="polite">
      <p class="text-sm text-danger">搜索出错</p>
      <p class="max-w-md text-xs text-muted">{search.filesError}</p>
    </div>
  {:else if search.status?.root}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-fg">输入文件名开始查找</p>
      <p class="text-xs text-muted">支持模糊匹配</p>
    </div>
  {/if}
</section>
