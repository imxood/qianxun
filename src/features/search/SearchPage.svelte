<script lang="ts">
  import FilesPage from './FilesPage.svelte';
  import GrepPage from './GrepPage.svelte';
  import DiskScan from './disk/DiskScan.svelte';

  type TabId = 'files' | 'grep' | 'disk';

  const tabs: Array<{ id: TabId; label: string }> = [
    { id: 'files', label: '找文件' },
    { id: 'grep', label: '搜内容' },
    { id: 'disk', label: '磁盘扫描' },
  ];

  // 页内三功能各自保活：切签只显隐，搜索结果与扫描进度全程不丢。
  let tab = $state<TabId>('files');

  const show = (id: TabId): string => (tab === id ? '' : 'invisible');
</script>

<section class="flex h-full flex-col">
  <header class="flex shrink-0 items-center gap-4 px-6 pb-3 pt-5">
    <h1 class="qx-page-title">文件</h1>
    <div
      class="flex items-center gap-0.5 rounded-lg border border-line bg-surface p-1"
      role="tablist"
      aria-label="文件工具"
    >
      {#each tabs as item (item.id)}
        <button
          role="tab"
          aria-selected={tab === item.id}
          data-testid="files-tab-{item.id}"
          class="qx-segment {tab === item.id ? 'qx-segment-on' : 'qx-segment-off'}"
          onclick={() => (tab = item.id)}
        >
          {item.label}
        </button>
      {/each}
    </div>
  </header>

  <div class="relative min-h-0 flex-1">
    <div class="absolute inset-0 overflow-y-auto px-6 pb-6 {show('files')}">
      <FilesPage />
    </div>
    <div class="absolute inset-0 overflow-y-auto px-6 pb-6 {show('grep')}">
      <GrepPage />
    </div>
    <div class="absolute inset-0 px-6 pb-6 {show('disk')}">
      <DiskScan />
    </div>
  </div>
</section>
