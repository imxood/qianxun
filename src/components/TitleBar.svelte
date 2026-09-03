<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';

  // 自绘标题栏（窗口 decorations: false）。三个窗口按钮只调用窗口 API，
  // 真正的关闭语义（隐藏到托盘还是退出）由 Rust 侧 CloseRequested 处理器决定。
  const win = getCurrentWindow();
</script>

<header
  class="flex h-8 shrink-0 select-none items-stretch justify-between border-b border-line bg-surface"
  data-tauri-drag-region
>
  <div class="flex items-baseline gap-2 self-center pl-3" data-tauri-drag-region>
    <span class="text-sm font-semibold tracking-wide">千寻</span>
    <span class="text-xs text-muted">Qianxun</span>
  </div>
  <!-- 双击最大化挂在独立占位元素上并声明交互角色：拖拽区保持「静态」，
       不给读屏软件制造无法解释的交互。 -->
  <div
    class="flex-1"
    data-tauri-drag-region
    role="button"
    tabindex="-1"
    aria-label="双击最大化或还原窗口"
    ondblclick={() => void win.toggleMaximize()}
  ></div>
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
