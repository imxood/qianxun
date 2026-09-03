<script lang="ts">
  import { search } from '../../stores/search.svelte';
  import { settings } from '../../stores/settings.svelte';

  // 草稿可编辑；根目录切换成功后由 store 回填（writable derived 的双向同步）。
  let draft = $state('');

  const history = $derived(settings.current?.search.rootHistory ?? []);

  function submit(): Promise<void> {
    return search.open(draft.trim()).then(() => void remember());
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
  <div class="flex gap-2">
    <input
      class="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-1.5 text-sm"
      type="text"
      list="search-root-history"
      placeholder="搜索根目录（绝对路径）"
      value={search.status?.root ?? draft}
      oninput={(event) => (draft = event.currentTarget.value)}
      onkeydown={(event) => {
        if (event.key === 'Enter') void submit();
      }}
    />
    <datalist id="search-root-history">
      {#each history as item (item)}
        <option value={item}></option>
      {/each}
    </datalist>
    <button
      class="shrink-0 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white transition-colors hover:bg-accent/90"
      onclick={() => void submit()}
    >
      打开
    </button>
  </div>
  <p class="text-xs text-muted">
    {#if search.openError}
      <span class="text-danger">{search.openError}</span>
    {:else if search.status?.root}
      {search.status.root} ·
      {#if search.scanning}
        <span class="text-accent">索引扫描中…（已见 {search.status.files} 个文件）</span>
      {:else}
        {search.status.files} 个文件{search.status.watcherReady ? ' · 实时监听中' : ''}
      {/if}
    {:else}
      尚未选择根目录。输入一个目录并回车，索引在后台建立。
    {/if}
  </p>
</div>
