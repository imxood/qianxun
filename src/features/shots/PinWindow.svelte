<script lang="ts">
  /**
   * 贴图 Pin 窗（#/pin?path=…）：置顶小窗显示截图。
   * 拖动 = 窗口移动（drag region）；滚轮缩放；双击 / Esc 关闭。
   */
  import { onMount } from 'svelte';
  import { convertFileSrc } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  const params = new URLSearchParams(window.location.hash.split('?')[1] ?? '');
  const imagePath = params.get('path') ?? '';
  const current = getCurrentWindow();

  let image = $state<HTMLImageElement | null>(null);
  let scale = $state(1);
  let dpr = 1;

  onMount(() => {
    dpr = window.devicePixelRatio || 1;
    const element = new Image();
    element.onload = async () => {
      image = element;
      // 初始尺寸 = 图片逻辑尺寸（cap 到屏幕 70%）。
      const logicalW = element.naturalWidth / dpr;
      const logicalH = element.naturalHeight / dpr;
      const factor = Math.min(
        1,
        (window.screen.availWidth * 0.7) / logicalW,
        (window.screen.availHeight * 0.7) / logicalH,
      );
      scale = factor;
      await current.setSize(
        new (await import('@tauri-apps/api/dpi')).LogicalSize(
          Math.max(80, logicalW * factor),
          Math.max(60, logicalH * factor),
        ),
      );
      await current.show();
    };
    element.src = convertFileSrc(imagePath);

    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void current.close();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function onWheel(event: WheelEvent): void {
    event.preventDefault();
    scale = Math.min(8, Math.max(0.1, scale * (event.deltaY < 0 ? 1.1 : 0.9)));
  }

  function naturalSize(): { width: number; height: number } {
    const width = image?.naturalWidth ?? 0;
    const height = image?.naturalHeight ?? 0;
    return { width: width / dpr, height: height / dpr };
  }
</script>

<div
  class="fixed inset-0 flex items-center justify-center overflow-hidden"
  role="button"
  aria-label="贴图：双击或按 Esc 关闭，滚轮缩放，拖动移动"
  data-tauri-drag-region
  onwheel={onWheel}
  ondblclick={() => void current.close()}
>
  {#if image}
    <img
      src={convertFileSrc(imagePath)}
      alt="贴图"
      class="pointer-events-none block select-none"
      style="width: {naturalSize().width * scale}px; height: {naturalSize().height * scale}px;"
      draggable="false"
    />
  {/if}
</div>
