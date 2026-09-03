<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { sliceByByteOffsets } from '../../lib/ipc/contract';
  import { locateInExplorer } from './locate';
  import RootBar from './RootBar.svelte';
</script>

<section class="space-y-4">
  <RootBar />

  <div class="flex items-center gap-2">
    <input
      class="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-2 text-sm"
      type="text"
      placeholder={search.status?.root ? '输入文件名，模糊匹配…' : '先选择根目录'}
      disabled={!search.status?.root}
      bind:value={search.filesQuery}
      oninput={() => search.scheduleFiles()}
    />
    {#if search.filesBusy}<span class="text-xs text-accent">搜索中…</span>{/if}
  </div>

  {#if search.filesResult}
    <p class="text-xs text-muted">
      命中 {search.filesResult.totalMatched} / 索引 {search.filesResult.totalFiles} 个文件 （显示前 {search
        .filesResult.items.length} 条，按相关度排序）
    </p>
    <ul class="divide-y divide-line rounded-lg border border-line bg-card">
      {#each search.filesResult.items as hit (hit.path)}
        <li class="flex items-baseline justify-between gap-3 px-3 py-2">
          <span class="min-w-0 break-all font-mono text-sm">
            {#each sliceByByteOffsets(hit.path, hit.offsets) as segment, segmentIndex (hit.path + segmentIndex)}
              {#if segment.matched}<mark class="rounded bg-accent-soft">{segment.text}</mark
                >{:else}{segment.text}{/if}
            {/each}
          </span>
          <span class="flex shrink-0 items-center gap-2 text-xs text-muted">
            {hit.score}
            <button class="hover:text-fg" onclick={() => locateInExplorer(hit.path)}> 定位 </button>
          </span>
        </li>
      {/each}
    </ul>
  {:else if search.status?.root && search.filesQuery}
    <p class="text-sm text-muted">无结果</p>
  {/if}
</section>
