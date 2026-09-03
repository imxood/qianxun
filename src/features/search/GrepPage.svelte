<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { sliceByByteOffsets, type GrepHit } from '../../lib/ipc/contract';
  import { locateInExplorer } from './locate';
  import RootBar from './RootBar.svelte';
  import Switch from '../../components/Switch.svelte';

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

  function line(hit: GrepHit): string {
    return `${hit.lineNumber}`;
  }
</script>

<section class="space-y-4">
  <RootBar />

  <div class="flex items-center gap-2">
    <input
      class="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-2 font-mono text-sm"
      type="text"
      placeholder={search.status?.root ? '输入要查找的内容，回车搜索…' : '先选择根目录'}
      disabled={!search.status?.root}
      bind:value={search.grepQuery}
      onkeydown={(event) => {
        if (event.key === 'Enter') void search.runGrep();
      }}
    />
    <button
      class="shrink-0 rounded-md bg-accent px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
      disabled={!search.status?.root || !search.grepQuery.trim() || search.grepBusy}
      onclick={() => void search.runGrep()}
    >
      搜索
    </button>
    {#if search.grepBusy}
      <button
        class="shrink-0 rounded-md border border-line px-3 py-2 text-sm transition-colors hover:bg-accent-soft"
        onclick={() => void search.cancelGrep()}
      >
        取消
      </button>
    {/if}
  </div>

  <div
    class="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-lg border border-line bg-card px-3 py-2 text-xs text-muted"
  >
    <label class="flex items-center gap-2">
      正则
      <Switch
        label="正则模式"
        checked={search.grepOptions.regex}
        onchange={(value) => (search.grepOptions = { ...search.grepOptions, regex: value })}
      />
    </label>
    <label class="flex items-center gap-2">
      智能大小写
      <Switch
        label="智能大小写"
        checked={search.grepOptions.smartCase}
        onchange={(value) => (search.grepOptions = { ...search.grepOptions, smartCase: value })}
      />
    </label>
    <label class="flex items-center gap-1.5">
      前
      <input
        class="w-12 rounded border border-line bg-surface px-1.5 py-0.5 text-right"
        type="number"
        min="0"
        max="10"
        bind:value={search.grepOptions.beforeContext}
      />
      行
    </label>
    <label class="flex items-center gap-1.5">
      后
      <input
        class="w-12 rounded border border-line bg-surface px-1.5 py-0.5 text-right"
        type="number"
        min="0"
        max="10"
        bind:value={search.grepOptions.afterContext}
      />
      行
    </label>
  </div>

  {#if search.grepResult}
    <p class="text-xs text-muted">
      已搜 {search.grepResult.filesSearched} 个文件 ·
      {search.grepResult.filesWithMatches} 个文件命中 ·
      {search.grepResult.items.length} 条匹配
      {#if search.grepResult.aborted}<span class="text-danger">（已取消）</span>{/if}
    </p>

    <div class="space-y-3">
      {#each groups as [path, hits] (path)}
        <div class="overflow-hidden rounded-lg border border-line bg-card">
          <p class="border-b border-line bg-surface px-3 py-1.5 font-mono text-xs text-muted">
            {path}
            <span class="ml-2 rounded bg-accent-soft px-1.5 py-0.5">{hits.length} 处</span>
            <button class="ml-2 hover:text-fg" onclick={() => locateInExplorer(path)}>定位</button>
          </p>
          <div class="divide-y divide-line/50">
            {#each hits as hit, hitIndex (hit.path + hit.lineNumber + hit.col + hitIndex)}
              <div class="px-3 py-1.5 font-mono text-xs leading-relaxed">
                {#each hit.contextBefore as context, contextIndex (hit.lineNumber + '-b' + contextIndex)}
                  <p class="text-muted/70">{context}</p>
                {/each}
                <p>
                  <span class="mr-2 inline-block w-10 shrink-0 select-none text-right text-muted">
                    {line(hit)}
                  </span>
                  {#each sliceByByteOffsets(hit.lineContent, hit.offsets) as segment, segmentIndex (hit.lineNumber + '-s' + segmentIndex)}
                    {#if segment.matched}<mark class="rounded bg-accent-soft">{segment.text}</mark
                      >{:else}{segment.text}{/if}
                  {/each}
                </p>
                {#each hit.contextAfter as context, contextIndex (hit.lineNumber + '-a' + contextIndex)}
                  <p class="text-muted/70">{context}</p>
                {/each}
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>

    {#if search.grepResult.nextFileOffset > 0}
      <button
        class="rounded-md border border-line px-3 py-1.5 text-sm transition-colors hover:bg-accent-soft disabled:opacity-50"
        disabled={search.grepBusy}
        onclick={() => void search.runGrep(true)}
      >
        继续搜索后续文件…
      </button>
    {/if}
  {:else if search.status?.root}
    <p class="text-sm text-muted">输入内容后回车。上下文行数、正则与大小写策略在上方调整。</p>
  {/if}
</section>
