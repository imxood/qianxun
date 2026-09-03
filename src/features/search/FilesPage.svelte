<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { fileIconClass, FILE_ICON_PATH, highlightName, splitHighlightedPath } from './fileIcon';
  import { locateInExplorer, openFile } from './locate';
  import RootBar from './RootBar.svelte';
</script>

<section class="mx-auto w-full max-w-4xl space-y-4">
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
        placeholder={search.status?.root ? '按文件名过滤，即时生效' : '先选择目录'}
        disabled={!search.status?.root}
        bind:value={search.filesQuery}
        oninput={() => search.scheduleFiles()}
        onkeydown={(event) => {
          if (event.key === 'Escape') {
            search.filesQuery = '';
            search.filesResult = null;
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
      <span class="shrink-0 text-xs text-muted">{search.filesResult.totalMatched} 个结果</span>
    {/if}
  </div>

  {#if search.filesResult && search.filesResult.items.length > 0}
    <div class="divide-y divide-line/60 overflow-hidden rounded-lg border border-line bg-card">
      {#each search.filesResult.items as hit (hit.path)}
        {@const { directory, name, nameOffsets } = splitHighlightedPath(hit.path, hit.offsets)}
        <div class="group flex items-center transition-colors hover:bg-accent-soft/40">
          <button
            class="flex min-w-0 flex-1 items-center gap-3 px-3 py-2 text-left"
            title={hit.path}
            onclick={() => openFile(hit.path)}
          >
            <svg
              viewBox="0 0 24 24"
              class="size-4.5 shrink-0 {fileIconClass(hit.path)}"
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
              <span class="text-muted/70">{directory}</span
              >{#each highlightName(name, nameOffsets) as segment, index (hit.path + index)}{#if segment.matched}<mark
                    class="rounded bg-accent-soft text-fg">{segment.text}</mark
                  >{:else}{segment.text}{/if}{/each}
            </span>
          </button>
          <button
            class="mr-3 shrink-0 rounded px-1.5 py-0.5 text-xs text-muted opacity-0 transition-all hover:bg-accent-soft hover:text-fg group-hover:opacity-100"
            onclick={() => locateInExplorer(hit.path)}
          >
            定位
          </button>
        </div>
      {/each}
    </div>
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
