<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window';

  // 自绘标题栏（窗口 decorations: false）。三个窗口按钮只调用窗口 API，
  // 真正的关闭语义（隐藏到托盘还是退出）由 Rust 侧 CloseRequested 处理器决定。
  //
  // 拖动与双击最大化全部由 Tauri 内置的 data-tauri-drag-region 脚本接管
  // （mousedown → start_dragging；detail=2 → internal_toggle_maximize）。
  // 不要再在元素上挂 ondblclick 切换最大化——会和内置行为双重切换
  // （切过去又切回来），表现为「双击无效果」。
  // data-tauri-drag-region="deep"：子树内任意位置按下都算拖拽起点
  // （按钮等可点元素会被内置脚本自动豁免，不影响点击）。
  const win = getCurrentWindow();
</script>

<header
  class="flex h-8 shrink-0 select-none items-stretch justify-between border-b border-line bg-surface"
  data-tauri-drag-region="deep"
>
  <div class="flex items-baseline gap-2 self-center pl-3">
    <span class="text-sm font-semibold tracking-wide">千寻</span>
    <span class="text-xs text-muted">Qianxun</span>
  </div>
  <!-- 中段留空作为拖拽区（继承 header 的 deep 规则），也承载双击最大化。 -->
  <div class="flex-1"></div>
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
