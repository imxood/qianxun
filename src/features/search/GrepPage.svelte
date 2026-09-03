<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { contextMenu, type MenuItem } from '../../lib/menu.svelte';
  import { sliceByByteOffsets, type GrepHit } from '../../lib/ipc/contract';
  import { fileIconClass, FILE_ICON_PATH, splitPath } from './fileIcon';
  import { absolutePath, copyMenuItems, copyText, locateInExplorer, openFile } from './locate';
  import RootBar from './RootBar.svelte';

  // 结果按文件分组渲染（一次搜索的命中天然按文件聚集）。
  const groups = $derived.by(() => {
    const map: Array<[string, GrepHit[]]> = [];
    for (const hit of search.grepResult?.items ?? []) {
      const existing = map.find(([path]) => path === hit.path);
      if (existing) {
        existing[1].push(hit);
      } else {
        map.push([hit.path, [hit]]);
      }
    }
    return map;
  });

  let collapsed = $state<string[]>([]);

  function toggleGroup(path: string): void {
    collapsed = collapsed.includes(path)
      ? collapsed.filter((item) => item !== path)
      : [...collapsed, path];
  }

  function clampContext(value: number): number {
    return Math.min(10, Math.max(0, Number.isNaN(value) ? 1 : Math.round(value)));
  }

  /** 上下文行数一个控件同时驱动前后上下文（产品语义上它们是一回事）。 */
  function setContext(value: number): void {
    const lines = clampContext(value);
    search.grepOptions = { ...search.grepOptions, beforeContext: lines, afterContext: lines };
  }

  /** 切一个开关（regex / smartCase / wholeWord），改完立即重搜。 */
  function flip(key: 'regex' | 'smartCase'): void {
    search.grepOptions = { ...search.grepOptions, [key]: !search.grepOptions[key] };
    search.scheduleGrep();
  }

  function toggleWholeWord(): void {
    search.grepWholeWord = !search.grepWholeWord;
    search.scheduleGrep();
  }

  function setGlob(value: string): void {
    search.grepGlob = value;
    search.scheduleGrep();
  }

  function menuForFile(path: string): MenuItem[] {
    return [
      { label: '打开文件', onclick: () => openFile(path) },
      { label: '打开所在位置', onclick: () => locateInExplorer(path) },
      ...copyMenuItems(path),
    ];
  }

  function hitMenu(event: MouseEvent, hit: GrepHit): void {
    event.stopPropagation();
    contextMenu.show(event, [
      { label: '打开文件', onclick: () => openFile(hit.path) },
      {
        label: `复制 路径:${hit.lineNumber}`,
        onclick: () => void copyText(`${absolutePath(hit.path) ?? hit.path}:${hit.lineNumber}`),
      },
      ...copyMenuItems(hit.path),
    ]);
  }
</script>

<section class="mx-auto w-full max-w-5xl space-y-4">
  <RootBar />

  <div class="flex items-center gap-2">
    <div
      class="flex min-w-0 flex-1 items-stretch overflow-hidden rounded-md border border-line bg-surface focus-within:ring-1 focus-within:ring-accent"
    >
      <svg
        viewBox="0 0 24 24"
        class="ml-3 size-4 shrink-0 self-center text-muted"
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
        class="min-w-0 flex-1 bg-transparent px-2.5 py-2 font-mono text-sm placeholder:text-muted/60 focus:outline-none"
        type="text"
        placeholder={search.status?.root ? '搜索文件内容（流式出结果，无需回车）' : '先选择目录'}
        disabled={!search.status?.root}
        bind:value={search.grepQuery}
        oninput={() => search.scheduleGrep()}
        onkeydown={(event) => {
          if (event.key === 'Enter') void search.runGrep();
          if (event.key === 'Escape') {
            search.grepQuery = '';
            search.grepResult = null;
          }
        }}
      />
      <button
        class="border-l border-line px-2.5 font-mono text-xs transition-colors {search.grepOptions
          .regex
          ? 'bg-accent-soft text-accent'
          : 'text-muted hover:text-fg'}"
        title="正则表达式"
        aria-pressed={search.grepOptions.regex}
        onclick={() => flip('regex')}
      >
        .*
      </button>
      <button
        class="border-l border-line px-2.5 text-xs transition-colors {search.grepOptions.smartCase
          ? 'bg-accent-soft text-accent'
          : 'text-muted hover:text-fg'}"
        title="智能大小写（全小写时忽略大小写）"
        aria-pressed={search.grepOptions.smartCase}
        onclick={() => flip('smartCase')}
      >
        Aa
      </button>
      <button
        class="border-l border-line px-2.5 font-mono text-xs transition-colors {search.grepWholeWord
          ? 'bg-accent-soft text-accent'
          : 'text-muted hover:text-fg'}"
        title="整词匹配"
        aria-pressed={search.grepWholeWord}
        onclick={toggleWholeWord}
      >
        w
      </button>
    </div>

    <label class="flex shrink-0 items-center gap-1.5 text-xs text-muted">
      上下文
      <input
        class="w-12 rounded border border-line bg-surface px-1.5 py-1 text-right text-xs"
        type="number"
        min="0"
        max="10"
        value={search.grepOptions.beforeContext}
        onchange={(event) => setContext(Number(event.currentTarget.value))}
      />
    </label>
    {#if search.grepBusy}
      <button
        class="shrink-0 rounded-md border border-line px-2.5 py-1.5 text-xs transition-colors hover:bg-accent-soft"
        onclick={() => void search.cancelGrep()}
      >
        停止
      </button>
    {/if}
  </div>

  <div class="flex items-center gap-2 text-xs">
    <input
      class="w-56 rounded border border-line bg-surface px-2 py-1 font-mono text-xs placeholder:text-muted/60"
      type="text"
      placeholder="文件过滤，如 *.rs 或 src/**"
      value={search.grepGlob}
      oninput={(event) => setGlob(event.currentTarget.value)}
    />
    {#if search.grepBusy}
      <span class="flex items-center gap-1.5 text-accent">
        <span
          class="inline-block size-3 animate-spin rounded-full border-2 border-line border-t-accent"
          aria-hidden="true"
        ></span>
        已扫描 {search.grepResult?.filesSearched ?? 0} 个文件
      </span>
    {:else if search.grepResult}
      <span class="text-muted">
        {search.grepResult.filesWithMatches} 个文件 · {search.grepResult.items.length} 处命中
      </span>
    {/if}
    {#if search.grepError}
      <span class="text-danger">{search.grepError}</span>
    {:else if search.grepResult?.aborted}
      <span class="text-muted">已停止（取消或达到 2000 条上限——试试缩小范围或加文件过滤）</span>
    {/if}
  </div>

  {#if groups.length > 0}
    <div class="divide-y divide-line/60 overflow-hidden rounded-lg border border-line bg-card">
      {#each groups as [path, hits] (path)}
        {@const { directory, name } = splitPath(path)}
        <div class="group">
          <button
            class="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-accent-soft/40"
            title={path}
            onclick={() => toggleGroup(path)}
            oncontextmenu={(event) => contextMenu.show(event, menuForFile(path))}
          >
            <svg
              viewBox="0 0 24 24"
              class="size-4 shrink-0 {fileIconClass(path)}"
              fill="none"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d={FILE_ICON_PATH} />
            </svg>
            <span class="min-w-0 truncate font-mono text-sm">
              <span class="text-muted/70">{directory}</span><span class="text-fg">{name}</span>
            </span>
            <span class="ml-auto shrink-0 text-xs text-muted">{hits.length}</span>
            <svg
              viewBox="0 0 24 24"
              class="size-3.5 shrink-0 text-muted transition-transform {collapsed.includes(path)
                ? ''
                : 'rotate-180'}"
              fill="none"
              stroke="currentColor"
              stroke-width="1.6"
              aria-hidden="true"
            >
              <path d="M6 9l6 6 6-6" />
            </svg>
          </button>
          {#if !collapsed.includes(path)}
            <div class="pb-2">
              {#each hits as hit (path + hit.lineNumber)}
                <button
                  class="block w-full px-3 py-0.5 text-left transition-colors hover:bg-accent-soft/30"
                  onclick={() => openFile(hit.path)}
                  oncontextmenu={(event) => hitMenu(event, hit)}
                >
                  <span class="w-12 inline-block text-right font-mono text-xs text-muted/60"
                    >{hit.lineNumber}</span
                  >
                  <span class="ml-2 font-mono text-sm text-fg">
                    {#each sliceByByteOffsets(hit.lineContent, hit.offsets) as segment, i (path + hit.lineNumber + i)}{#if segment.matched}<mark
                          class="rounded bg-accent-soft px-0.5 text-fg">{segment.text}</mark
                        >{:else}{segment.text}{/if}{/each}
                  </span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {:else if search.grepBusy}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-muted">正在扫描…结果流式到达</p>
    </div>
  {:else if search.grepResult}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-fg">无匹配内容</p>
      <p class="text-xs text-muted">试试 .* 正则、w 整词或放宽文件过滤</p>
    </div>
  {:else if search.status?.root}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-fg">输入内容开始搜索</p>
      <p class="text-xs text-muted">.* 正则 · Aa 智能大小写 · w 整词 · 文件过滤 glob · 即输即搜</p>
    </div>
  {/if}
</section>
