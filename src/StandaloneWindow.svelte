<script lang="ts">
  /**
   * 独立窗口壳：终端 / DSH 页分离后的宿主。自绘标题栏（拖拽区 +
   * 最小化/最大化/关闭），关闭请求经 Rust 拦截转交到这里确认——
   * 终端窗口有活动会话时弹确认（固定会话按「进程退出」语义保留记录）。
   *
   * 「合并到主窗口」：把本窗口名下全部存活会话 terminal_transfer 给
   * 主窗，再前置主窗、关闭自己——等价于浏览器「把标签拖回原窗口」。
   */
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { call } from './lib/ipc';
  import { standaloneView, WINDOW_LABEL } from './lib/windowEnv';
  import { settings } from './stores/settings.svelte';
  import { theme } from './stores/theme.svelte';
  import { shellTitle } from './lib/utils/shell';
  import ConfirmDialog from './components/ConfirmDialog.svelte';
  import TerminalPage from './features/terminal/TerminalPage.svelte';
  import DshPage from './features/dsh/DshPage.svelte';
  import type { TerminalSessionSnapshot } from './lib/ipc/contract';

  const win = getCurrentWindow();
  const view = standaloneView();

  const meta: Record<string, { title: string }> = {
    terminal: { title: '终端 · 千寻' },
    dsh: { title: 'DSH · 千寻' },
  };

  // 设置到达后同步主题（启动时与每次保存后各一次），与主窗同一数据流。
  $effect(() => {
    if (settings.current) theme.set(settings.current.theme);
  });
  $effect(() => {
    document.documentElement.classList.toggle('dark', theme.resolved === 'dark');
    try {
      localStorage.setItem('qx-theme', theme.resolved);
    } catch {
      /* 隐私模式等：回写失败无碍 */
    }
  });

  let closeTarget: number | null = $state(null);
  let merging = $state(false);

  onMount(() => {
    // 窗口以隐藏创建（window_spawn_view visible:false），首帧就绪后亮出。
    void settings.load().finally(() => {
      void win.show();
      void win.setFocus();
    });
    // 任务栏/系统关闭请求由 Rust 拦截后转发到这里走确认流。
    const disposers: Array<() => void> = [];
    void import('@tauri-apps/api/event').then(({ listen }) =>
      listen('window://close-requested', () => void requestClose()).then((unlisten) =>
        disposers.push(unlisten),
      ),
    );
    return () => {
      for (const dispose of disposers) dispose();
    };
  });

  /** 关闭入口（标题栏 × 与系统关闭请求共用）：终端有活动会话先确认。 */
  async function requestClose(): Promise<void> {
    if (view === 'terminal') {
      try {
        const sessions = await call<TerminalSessionSnapshot[]>('terminal_sessions', {
          label: WINDOW_LABEL,
        });
        if (sessions.length > 0) {
          closeTarget = sessions.length;
          return;
        }
      } catch {
        // 查询失败按无会话处理：直接关（Rust 侧 Destroyed 仍会兜底清理）。
      }
    }
    void forceClose();
  }

  function forceClose(): void {
    closeTarget = null;
    void call('window_force_close').catch(() => {});
  }

  /** 合并到主窗口：全部会话转移 → 前置主窗 → 关闭自己。 */
  async function mergeToMain(): Promise<void> {
    if (view !== 'terminal' || merging) return;
    merging = true;
    try {
      const sessions = await call<TerminalSessionSnapshot[]>('terminal_sessions', {
        label: WINDOW_LABEL,
      });
      for (const session of sessions) {
        await call('terminal_transfer', {
          id: session.id,
          target: 'main',
          title: session.title ?? shellTitle(session.shell),
          shell: session.shell,
          cwd: session.cwd,
          pinId: session.pinId,
        });
      }
      await call('window_reveal_main');
      await call('window_force_close');
    } catch {
      merging = false; // 转移失败：留在本窗口，用户可重试或逐标签处理。
    }
  }
</script>

<div class="flex h-full flex-col overflow-hidden bg-bg">
  <header
    class="flex h-8 shrink-0 select-none items-stretch justify-between border-b border-line bg-surface"
    data-tauri-drag-region="deep"
  >
    <div class="flex items-center gap-2 self-center pl-3">
      <span class="text-xs font-medium">{meta[view ?? '']?.title ?? '千寻'}</span>
      <span class="text-[10px] text-muted">独立窗口</span>
    </div>
    <div class="flex items-stretch">
      {#if view === 'terminal'}
        <button
          class="flex items-center px-2.5 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
          title="把本窗口的全部终端合并回主窗口"
          disabled={merging}
          onclick={() => void mergeToMain()}
        >
          {merging ? '合并中…' : '合并到主窗口'}
        </button>
      {/if}
      <button
        class="flex w-11 items-center justify-center text-fg transition-colors hover:bg-accent-soft"
        aria-label="最小化"
        onclick={() => void win.minimize()}
      >
        <svg
          viewBox="0 0 24 24"
          class="size-3.5"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
        >
          <path d="M5 12h14" />
        </svg>
      </button>
      <button
        class="flex w-11 items-center justify-center text-fg transition-colors hover:bg-accent-soft"
        aria-label="最大化 / 还原"
        onclick={() => void win.toggleMaximize()}
      >
        <svg
          viewBox="0 0 24 24"
          class="size-3.5"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
        >
          <rect x="6" y="6" width="12" height="12" rx="1" />
        </svg>
      </button>
      <button
        class="flex w-11 items-center justify-center text-fg transition-colors hover:bg-danger hover:text-white"
        aria-label="关闭"
        onclick={() => void requestClose()}
      >
        <svg
          viewBox="0 0 24 24"
          class="size-3.5"
          fill="none"
          stroke="currentColor"
          stroke-width="1.6"
        >
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </button>
    </div>
  </header>

  <div class="min-h-0 flex-1">
    {#if view === 'terminal'}
      <TerminalPage standalone />
    {:else if view === 'dsh'}
      <DshPage standalone />
    {:else}
      <div class="flex h-full items-center justify-center text-sm text-muted">未知视图</div>
    {/if}
  </div>
</div>

<ConfirmDialog
  open={closeTarget !== null}
  title="关闭独立终端窗口"
  message={closeTarget !== null
    ? `本窗口还有 ${closeTarget} 个终端在运行，关闭将结束这些进程（已固定的会话保留记录，下次启动可恢复）。确定关闭？`
    : ''}
  confirmLabel="结束并关闭"
  danger
  onconfirm={forceClose}
  oncancel={() => (closeTarget = null)}
/>
