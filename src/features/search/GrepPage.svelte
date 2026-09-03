<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { sliceByByteOffsets, type GrepHit } from '../../lib/ipc/contract';
  import { fileIconClass, FILE_ICON_PATH, splitPath } from './fileIcon';
  import { locateInExplorer } from './locate';
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

  function line(hit: GrepHit): string {
    return `${hit.lineNumber}`;
  }

  function clampContext(value: number): number {
    return Math.min(10, Math.max(0, Number.isNaN(value) ? 1 : Math.round(value)));
  }

  /** 上下文行数一个控件同时驱动前后上下文（产品语义上它们是一回事）。 */
  function setContext(value: number): void {
    const lines = clampContext(value);
    search.grepOptions = { ...search.grepOptions, beforeContext: lines, afterContext: lines };
  }
</script>

<section class="mx-auto w-full max-w-4xl space-y-4">
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
        placeholder={search.status?.root ? '搜索文件内容，回车确认' : '先选择目录'}
        disabled={!search.status?.root}
        bind:value={search.grepQuery}
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
        onclick={() =>
          (search.grepOptions = { ...search.grepOptions, regex: !search.grepOptions.regex })}
      >
        .*
      </button>
      <button
        class="border-l border-line px-2.5 text-xs transition-colors {search.grepOptions.smartCase
          ? 'bg-accent-soft text-accent'
          : 'text-muted hover:text-fg'}"
        title="智能大小写（全小写时忽略大小写）"
        aria-pressed={search.grepOptions.smartCase}
        onclick={() =>
          (search.grepOptions = {
            ...search.grepOptions,
            smartCase: !search.grepOptions.smartCase,
          })}
      >
        Aa
      </button>
    </div>

    <label class="flex shrink-0 items-center gap-1.5 text-xs text-muted">
      上下文
      <input
        class="w-12 rounded border border-line bg-surface px-1.5 py-1 text-right"
        type="number"
        min="0"
        max="10"
        value={search.grepOptions.beforeContext}
        onchange={(event) => setContext(Number(event.currentTarget.value))}
      />
      行
    </label>

    {#if search.grepBusy}
      <button
        class="shrink-0 rounded-md border border-line px-3 py-2 text-sm transition-colors hover:bg-accent-soft"
        onclick={() => void search.cancelGrep()}
      >
        取消
      </button>
    {:else}
      <button
        class="shrink-0 rounded-md bg-accent px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
        disabled={!search.status?.root || !search.grepQuery.trim()}
        onclick={() => void search.runGrep()}
      >
        搜索
      </button>
    {/if}
  </div>

  {#if search.grepResult}
    <div class="flex items-center gap-3 text-xs text-muted">
      <span>
        {search.grepResult.filesWithMatches} 个文件 · {search.grepResult.items.length} 条匹配
      </span>
      {#if search.grepBusy}
        <span class="flex items-center gap-1 text-accent">
          <span
            class="inline-block size-3 animate-spin rounded-full border-2 border-line border-t-accent"
            aria-hidden="true"
          ></span>
          搜索中…
        </span>
      {/if}
      {#if search.grepResult.aborted}
        <span class="text-danger">已取消</span>
      {/if}
    </div>

    <div class="space-y-3">
      {#each groups as [path, hits] (path)}
        {@const isCollapsed = collapsed.includes(path)}
        {@const parts = splitPath(path)}
        <div class="overflow-hidden rounded-lg border border-line bg-card">
          <div class="flex items-center gap-2 border-b border-line bg-surface px-3 py-1.5">
            <button
              class="flex min-w-0 flex-1 items-center gap-2 text-left"
              onclick={() => toggleGroup(path)}
              aria-expanded={!isCollapsed}
            >
              <svg
                viewBox="0 0 24 24"
                class="size-3.5 shrink-0 text-muted transition-transform {isCollapsed
                  ? ''
                  : 'rotate-90'}"
                fill="none"
                stroke="currentColor"
                stroke-width="1.8"
                stroke-linecap="round"
                aria-hidden="true"
              >
                <path d="M9 6l6 6-6 6" />
              </svg>
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
              <span class="truncate font-mono text-xs text-fg" title={path}>
                {#if parts.directory}<span class="text-muted/70">{parts.directory}</span
                  >{/if}{parts.name}
              </span>
            </button>
            <span class="shrink-0 rounded bg-accent-soft px-1.5 py-0.5 text-xs text-muted">
              {hits.length}
            </span>
            <button
              class="shrink-0 text-xs text-muted transition-colors hover:text-fg"
              onclick={() => locateInExplorer(path)}
            >
              定位
            </button>
          </div>
          {#if !isCollapsed}
            <div class="divide-y divide-line/50">
              {#each hits as hit, hitIndex (hit.path + hit.lineNumber + hit.col + hitIndex)}
                <div class="px-3 py-1 font-mono text-xs leading-relaxed">
                  {#each hit.contextBefore as context, contextIndex (hit.lineNumber + '-b' + contextIndex)}
                    <p class="whitespace-pre-wrap break-all pl-10 text-muted/60">{context}</p>
                  {/each}
                  <p class="whitespace-pre-wrap break-all">
                    <span class="inline-block w-8 select-none text-right text-muted"
                      >{line(hit)}</span
                    >
                    <span class="ml-2"
                      >{#each sliceByByteOffsets(hit.lineContent, hit.offsets) as segment, segmentIndex (hit.lineNumber + '-s' + segmentIndex)}{#if segment.matched}<mark
                            class="rounded bg-accent-soft text-fg">{segment.text}</mark
                          >{:else}{segment.text}{/if}{/each}</span
                    >
                  </p>
                  {#each hit.contextAfter as context, contextIndex (hit.lineNumber + '-a' + contextIndex)}
                    <p class="whitespace-pre-wrap break-all pl-10 text-muted/60">{context}</p>
                  {/each}
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/each}
    </div>

    {#if search.grepResult.nextFileOffset > 0}
      <div class="flex justify-center">
        <button
          class="rounded-md border border-line px-4 py-1.5 text-sm transition-colors hover:bg-accent-soft disabled:opacity-50"
          disabled={search.grepBusy}
          onclick={() => void search.runGrep(true)}
        >
          加载更多
        </button>
      </div>
    {/if}
  {:else if search.status?.root}
    <div class="flex flex-col items-center gap-1 py-16 text-center">
      <p class="text-sm text-fg">输入内容开始搜索</p>
      <p class="text-xs text-muted">.* 切换正则 · Aa 切换智能大小写 · 回车搜索</p>
    </div>
  {/if}
</section>
