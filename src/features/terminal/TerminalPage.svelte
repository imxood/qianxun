<script lang="ts">
  /**
   * 终端页（M4）：多标签宿主。标签条 + 全部 pane 常驻（CSS 隐藏保活）。
   * 运行中的标签关闭需确认；退出的标签自动移除。
   */
  import { onMount } from 'svelte';
  import { call } from '../../lib/ipc';
  import { settings } from '../../stores/settings.svelte';
  import TerminalPane from './TerminalPane.svelte';

  type Tab = { id: number; title: string; alive: boolean };

  let tabs = $state<Tab[]>([]);
  let activeId = $state<number | null>(null);

  const prefs = $derived(
    settings.current?.terminal ?? {
      shell: 'auto',
      fontSize: 13,
      scrollback: 5000,
    },
  );

  onMount(() => {
    void newTab();
  });

  async function newTab(): Promise<void> {
    try {
      const id = await call<number>('terminal_spawn', {
        shell: prefs.shell,
        cwd: null,
        cols: 80,
        rows: 24,
      });
      tabs = [...tabs, { id, title: `终端 ${id}`, alive: true }];
      activeId = id;
    } catch (error) {
      tabs = [...tabs, { id: -Date.now(), title: '启动失败', alive: false }];
      activeId = tabs[tabs.length - 1]?.id ?? null;
      console.error(error);
    }
  }

  function closeTab(tab: Tab): void {
    if (tab.alive && !window.confirm(`「${tab.title}」仍在运行，关闭将结束进程。确定？`)) {
      return;
    }
    void call('terminal_kill', { id: tab.id }).finally(() => remove(tab.id));
  }

  function remove(id: number): void {
    const index = tabs.findIndex((tab) => tab.id === id);
    if (index < 0) return;
    tabs = tabs.filter((tab) => tab.id !== id);
    if (activeId === id) {
      activeId = tabs[Math.min(index, tabs.length - 1)]?.id ?? null;
    }
  }

  function onPaneExit(id: number): void {
    // 进程退出：标记死亡，保留 pane 显示尾输出，标签条变灰。
    const tab = tabs.find((item) => item.id === id);
    if (tab) tab.alive = false;
  }

  function onPaneTitle(id: number, title: string): void {
    const tab = tabs.find((item) => item.id === id);
    if (tab && title.trim()) tab.title = title;
  }
</script>

<section class="flex h-full flex-col overflow-hidden">
  <div class="flex items-center gap-1 border-b border-line bg-surface px-2 py-1.5">
    {#each tabs as tab (tab.id)}
      <button
        class="group flex max-w-44 items-center gap-1.5 rounded-md px-2.5 py-1 text-xs transition-colors {activeId ===
        tab.id
          ? 'bg-accent-soft font-medium text-fg'
          : 'text-muted hover:bg-accent-soft/60'}"
        onclick={() => (activeId = tab.id)}
      >
        <span class="truncate {tab.alive ? '' : 'opacity-50 line-through'}">{tab.title}</span>
        <span
          class="ml-0.5 rounded px-1 text-muted/70 opacity-0 transition-opacity hover:bg-black/10 group-hover:opacity-100"
          role="button"
          tabindex="-1"
          aria-label="关闭标签"
          onclick={(event) => {
            event.stopPropagation();
            closeTab(tab);
          }}>×</span
        >
      </button>
    {/each}
    <button
      class="rounded-md px-2 py-1 text-sm text-muted transition-colors hover:bg-accent-soft hover:text-fg"
      title="新建终端"
      onclick={() => void newTab()}
    >
      +
    </button>
  </div>

  <div class="relative min-h-0 flex-1">
    {#each tabs as tab (tab.id)}
      <div class="absolute inset-0 {activeId === tab.id ? '' : 'hidden'}">
        {#if tab.id > 0}
          <TerminalPane id={tab.id} {prefs} onExit={onPaneExit} onTitle={onPaneTitle} />
        {/if}
      </div>
    {/each}
    {#if tabs.length === 0}
      <div class="flex h-full items-center justify-center text-sm text-muted">
        没有终端会话。点 + 新建。
      </div>
    {/if}
  </div>
</section>
