<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { search } from '../../stores/search.svelte';
  import { settings } from '../../stores/settings.svelte';
  import { formatBytes } from './format';

  // 草稿可编辑；根目录切换成功后由 store 回填（writable derived 的双向同步）。
  let draft = $state('');
  let historyOpen = $state(false);
  let picking = $state(false);

  const history = $derived(settings.current?.search.rootHistory ?? []);

  onMount(() => void search.loadDrives());

  const KIND_LABEL: Record<string, string> = {
    fixed: '本地磁盘',
    removable: '可移动',
    network: '网络',
    cdrom: '光驱',
    ramdisk: '内存盘',
  };

  /** 原生目录选择器：选完即打开索引，不再要求手输路径。 */
  async function pickDirectory(): Promise<void> {
    picking = true;
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === 'string' && selected.trim()) {
        await submit(selected);
      }
    } finally {
      picking = false;
    }
  }

  async function submit(root: string): Promise<void> {
    historyOpen = false;
    await search.open(root);
    await remember();
  }

  function submitDraft(): void {
    const root = draft.trim() || search.rootInput.trim();
    if (root) void submit(root);
  }

  /** 成功打开后把根目录记入历史（最近优先、去重、截断 8 条）。 */
  async function remember(): Promise<void> {
    const root = search.status?.root;
    if (!root || !settings.current) return;
    if (settings.current.search.rootHistory[0] === root) return;
    const next = [root, ...settings.current.search.rootHistory.filter((item) => item !== root)];
    try {
      await settings.update({ search: { rootHistory: next.slice(0, 8) } });
    } catch {
      // 历史记录失败不影响搜索本身。
    }
  }
</script>

<div class="space-y-2">
  <div class="flex items-center gap-2">
    <button
      class="flex shrink-0 items-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
      disabled={picking}
      onclick={() => void pickDirectory()}
    >
      <svg
        viewBox="0 0 24 24"
        class="size-4"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
      </svg>
      {picking ? '选择中…' : '选择目录'}
    </button>

    <div class="relative min-w-0 flex-1">
      <input
        class="w-full rounded-md border border-line bg-surface px-3 py-1.5 pr-8 text-sm placeholder:text-muted/60"
        type="text"
        placeholder="或粘贴目录路径（含整盘，如 D:\）"
        value={search.status?.root ?? draft}
        oninput={(event) => (draft = event.currentTarget.value)}
        onkeydown={(event) => {
          if (event.key === 'Enter') submitDraft();
          if (event.key === 'Escape') historyOpen = false;
        }}
      />
      {#if history.length > 0}
        <button
          class="absolute inset-y-0 right-0 flex w-8 items-center justify-center text-muted transition-colors hover:text-fg"
          aria-label="最近目录"
          aria-expanded={historyOpen}
          onclick={() => (historyOpen = !historyOpen)}
        >
          <svg
            viewBox="0 0 24 24"
            class="size-4"
            fill="none"
            stroke="currentColor"
            stroke-width="1.6"
          >
            <path d="M6 9l6 6 6-6" />
          </svg>
        </button>
        {#if historyOpen}
          <button
            class="fixed inset-0 z-10 cursor-default"
            aria-label="关闭历史列表"
            tabindex="-1"
            onclick={() => (historyOpen = false)}
          ></button>
          <div
            class="absolute inset-x-0 top-full z-20 mt-1 overflow-hidden rounded-md border border-line bg-card shadow-lg"
          >
            {#each history as item (item)}
              <button
                class="block w-full truncate px-3 py-2 text-left text-xs text-fg transition-colors hover:bg-accent-soft"
                title={item}
                onclick={() => void submit(item)}
              >
                {item}
              </button>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </div>

  {#if search.drives.length > 0}
    <div class="flex flex-wrap items-center gap-1.5">
      {#each search.drives as drive (drive.path)}
        <button
          class="rounded-full border border-line bg-surface px-2.5 py-0.5 text-xs text-fg transition-colors hover:border-accent hover:bg-accent-soft"
          title="{KIND_LABEL[drive.kind] ?? drive.kind} · 共 {formatBytes(
            drive.totalBytes,
          )} · 剩 {formatBytes(drive.freeBytes)}"
          onclick={() => void submit(drive.path)}
        >
          {drive.path}
          <span class="text-muted">{KIND_LABEL[drive.kind] ?? drive.kind}</span>
        </button>
      {/each}
    </div>
  {/if}

  <div class="flex h-5 items-center gap-2 text-xs">
    {#if search.openError}
      <span class="text-danger">{search.openError}</span>
    {:else if search.status?.root}
      <span class="rounded-full bg-accent-soft px-2 py-0.5 text-muted">
        已索引 {search.status.files} 个文件
      </span>
      {#if search.scanning}
        <span class="flex items-center gap-1 text-accent">
          <span
            class="inline-block size-3 animate-spin rounded-full border-2 border-line border-t-accent"
            aria-hidden="true"
          ></span>
          索引中…
        </span>
      {:else if search.status.watcherReady}
        <span class="text-muted">实时监听</span>
      {/if}
      <span class="truncate text-muted/70" title={search.status.root}>{search.status.root}</span>
    {:else}
      <span class="text-muted">点盘符或选目录后即可搜索文件与内容</span>
    {/if}
  </div>
</div>
