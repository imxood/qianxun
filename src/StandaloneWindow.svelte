<script lang="ts">
  /**
   * 独立窗口壳：DSH 页分离后的宿主。自绘标题栏（拖拽区 +
   * 最小化/最大化/关闭），关闭走原生流程——Destroyed 后 Rust 广播
   * window://closed，主窗恢复侧栏项。
   */
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { standaloneView } from './lib/windowEnv';
  import { settings } from './stores/settings.svelte';
  import { theme } from './stores/theme.svelte';
  import DshPage from './features/dsh/DshPage.svelte';

  const win = getCurrentWindow();
  const view = standaloneView();

  const meta: Record<string, { title: string }> = {
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

  onMount(() => {
    // 窗口以隐藏创建（window_spawn_view visible:false），首帧就绪后亮出。
    void settings.load().finally(() => {
      void win.show();
      void win.setFocus();
    });
  });
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
        onclick={() => void win.close()}
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
    {#if view === 'dsh'}
      <DshPage standalone />
    {:else}
      <div class="flex h-full items-center justify-center text-sm text-muted">未知视图</div>
    {/if}
  </div>
</div>
