<script lang="ts">
  /**
   * 终端页（M4）：多标签宿主。标签条 + 全部 pane 常驻（CSS 隐藏保活）。
   * 运行中的标签关闭需确认（自定义对话框——WebView2 原生 confirm 不可靠）；
   * 退出的标签保留尾输出并标灰；启动失败的标签显示原因并可重试。
   */
  import { onMount } from 'svelte';
  import { call } from '../../lib/ipc';
  import { settings } from '../../stores/settings.svelte';
  import ConfirmDialog from '../../components/ConfirmDialog.svelte';
  import TerminalPane from './TerminalPane.svelte';

  type Tab = { id: number; title: string; alive: boolean; error: string | null };

  let tabs = $state<Tab[]>([]);
  let activeId = $state<number | null>(null);
  let closeTarget: Tab | null = $state(null);

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

  /** 标签默认标题 = 解析后的 shell 名（pwsh / powershell / 自定义路径基名）。 */
  function shellTitle(shell: string): string {
    if (shell === 'auto') return '终端';
    const name = shell.replaceAll('\\', '/').split('/').pop() ?? shell;
    return name.replace(/\.exe$/i, '');
  }

  async function newTab(cwd: string | null = null): Promise<void> {
    try {
      const info = await call<{ id: number; shell: string }>('terminal_spawn', {
        shell: prefs.shell,
        cwd,
        cols: 80,
        rows: 24,
      });
      tabs = [...tabs, { id: info.id, title: shellTitle(info.shell), alive: true, error: null }];
      activeId = info.id;
    } catch (error) {
      // 失败原因上屏（错误窗格），不再只进 console。
      const message = error instanceof Error ? error.message : String(error);
      tabs = [...tabs, { id: -Date.now(), title: '启动失败', alive: false, error: message }];
      activeId = tabs[tabs.length - 1]?.id ?? null;
    }
  }

  function requestClose(tab: Tab): void {
    if (tab.alive) {
      closeTarget = tab;
      return;
    }
    void removeTab(tab);
  }

  function confirmClose(): void {
    const tab = closeTarget;
    closeTarget = null;
    if (tab) void removeTab(tab);
  }

  async function removeTab(tab: Tab): Promise<void> {
    try {
      await call('terminal_kill', { id: tab.id });
    } finally {
      remove(tab.id);
    }
  }

  function retry(tab: Tab): void {
    remove(tab.id);
    void newTab();
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
            requestClose(tab);
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
        {#if tab.error !== null}
          <div class="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
            <p class="text-sm text-fg">终端启动失败</p>
            <p class="max-w-md text-xs leading-5 text-muted">{tab.error}</p>
            <button
              class="rounded-md bg-accent-soft px-3 py-1.5 text-xs text-fg transition-colors hover:bg-accent-soft/70"
              onclick={() => retry(tab)}
            >
              重试
            </button>
          </div>
        {:else}
          <TerminalPane
            id={tab.id}
            active={activeId === tab.id}
            {prefs}
            onExit={onPaneExit}
            onTitle={onPaneTitle}
          />
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

<ConfirmDialog
  open={closeTarget !== null}
  title="关闭终端"
  message={closeTarget ? `「${closeTarget.title}」仍在运行，关闭将结束该 shell 的进程。确定？` : ''}
  confirmLabel="结束进程"
  danger
  onconfirm={confirmClose}
  oncancel={() => (closeTarget = null)}
/>
