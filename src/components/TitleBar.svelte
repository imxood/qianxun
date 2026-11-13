<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import {
    attachLinuxDrag,
    createLinuxMaximize,
    isLinuxRuntime,
    type LinuxMaximize,
  } from '../lib/window/linuxWindow';

  // 自绘标题栏（窗口 decorations: false）。三个窗口按钮只调用窗口 API，
  // 真正的关闭语义（隐藏到托盘还是退出）由 Rust 侧 CloseRequested 处理器决定。
  //
  // 拖动与双击最大化全部由 Tauri 内置的 data-tauri-drag-region 脚本接管
  // （mousedown → start_dragging；detail=2 → internal_toggle_maximize）。
  // 不要再在元素上挂 ondblclick 切换最大化——会和内置行为双重切换
  // （切过去又切回来），表现为「双击无效果」。
  // data-tauri-drag-region="deep"：子树内任意位置按下都算拖拽起点
  // （按钮等可点元素会被内置脚本自动豁免，不影响点击）。
  //
  // 例外：Linux（CEF 内嵌 Chromium）下内置脚本的 start_dragging 拿不到指针
  // 状态，拖不动窗口；最大化请求也会被 XWayland 忽略（无 WM frame 的 CSD 窗口）。
  // 于是 Linux 下这三件事全部由前端自己做：拖拽走 set_position，
  // 最大化走 set_size + set_position，双击由我们自己的 pointer 判定。
  // 详见 src/lib/window/linuxWindow.ts。
  const win = getCurrentWindow();

  let linux = $state(false);
  let headerEl: HTMLElement | undefined = $state();
  let maximize = $state<LinuxMaximize | null>(null);
  let maximized = $state(false);

  onMount(() => {
    void isLinuxRuntime().then((isLinux) => {
      linux = isLinux;
      if (!isLinux) return;
      const controller = createLinuxMaximize(win);
      maximize = controller;
      if (headerEl) {
        attachLinuxDrag(headerEl, win, {
          onDoubleClick: () => {
            void controller.toggle().then(() => {
              maximized = controller.isMaximized();
            });
          },
        });
      }
    });
  });

  // Linux 下 GTK4 在 XWayland 里图标化无效（WM_STATE 恒 Normal），
  // 最小化改为隐藏到托盘，与千寻既有的托盘行为一致。
  function minimize() {
    if (linux) {
      void win.hide();
      return;
    }
    void win.minimize();
  }

  // Linux 下最大化自己做（窗口管理器的 maximize 请求对 CSD 窗口无效）。
  function toggleMaximize() {
    if (linux && maximize) {
      void maximize.toggle().then(() => {
        maximized = maximize?.isMaximized() ?? false;
      });
      return;
    }
    void win.toggleMaximize();
  }
</script>

<header
  bind:this={headerEl}
  class="flex h-8 shrink-0 select-none items-stretch justify-between border-b border-line bg-surface"
  data-tauri-drag-region={linux ? 'false' : 'deep'}
>
  <div class="flex items-center self-center pr-3 pl-3">
    <span class="text-sm font-semibold tracking-tight">千寻</span>
  </div>
  <!-- 中段留空作为拖拽区（继承 header 的 deep 规则），也承载双击最大化。 -->
  <div class="flex-1"></div>
  <div class="flex items-stretch">
    <button
      class="flex w-11 items-center justify-center text-fg transition-colors hover:bg-accent-soft"
      aria-label="最小化"
      onclick={() => minimize()}
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
      onclick={() => toggleMaximize()}
    >
      <svg
        viewBox="0 0 24 24"
        class="size-3.5"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
      >
        {#if maximized}
          <path d="M9 9h6v6H9z" />
          <path d="M5 15V5h10" />
        {:else}
          <rect x="6" y="6" width="12" height="12" rx="1" />
        {/if}
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
