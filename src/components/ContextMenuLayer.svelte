<script lang="ts">
  import { contextMenu } from '../lib/menu.svelte';
  import {
    buildDefaultMenuItems,
    describeContextMenuContext,
  } from '../lib/window/defaultContextMenu';

  // 点击别处 / Esc / 滚动都收起菜单；右键其他目标由 show() 先 close 再开新。
  function dismiss(): void {
    contextMenu.close();
  }

  /**
   * 全局右键兜底：没有业务菜单时弹千寻自己的默认菜单（复制/粘贴/剪切/全选）。
   *
   * **为什么第一步必须 preventDefault**：Linux(CEF) 下 Chromium 的原生右键菜单是
   * 浏览器进程自绘的 popup 窗口，在 X11 里是独立 override-redirect 窗口；嵌在
   * 「GTK4 toplevel 套 CEF 子窗口」结构里时它被算成与主窗口等大（实测 1100x734，
   * 偏移正好是 _GTK_FRAME_EXTENTS），变成一块巨大的空白面板挡住界面，还可能被
   * 窗口管理器抢焦点后立刻消失（CEF issue #4329）。实测：渲染进程里对 contextmenu
   * 调 preventDefault，CEF 就不再创建那个弹窗。所以这里把 preventDefault 放在
   * 第一行，菜单项是空的就只压不弹。
   */
  function handleContextMenu(event: MouseEvent): void {
    // 业务菜单（侧边栏 / 搜索结果行 / 磁盘扫描等）自己监听 contextmenu 并调
    // contextMenu.show()，而 show() 内部会 preventDefault + stopPropagation，
    // 事件根本到不了 window → 这里不会重复弹菜单。
    // 万一业务只 preventDefault 而没 stopPropagation，用这个标志位兜底避免叠加。
    const handledByBusiness = event.defaultPrevented;
    event.preventDefault();
    if (handledByBusiness) return;
    const items = buildDefaultMenuItems(describeContextMenuContext(event.target));
    // 没有可编辑内容也没有选区：不弹菜单（但上面的 preventDefault 依然生效）。
    if (items.length === 0) return;
    contextMenu.show(event, items);
  }
</script>

<svelte:window
  onclick={dismiss}
  oncontextmenu={handleContextMenu}
  onkeydown={(event) => {
    if (event.key === 'Escape') dismiss();
  }}
  onblur={dismiss}
  onwheel={dismiss}
/>

{#if contextMenu.visible}
  <div
    class="fixed z-50 min-w-48 overflow-hidden rounded-md border border-line bg-card py-1 shadow-lg"
    style="left: {contextMenu.x}px; top: {contextMenu.y}px;"
    role="menu"
  >
    {#each contextMenu.items as item (item.label)}
      <button
        class="block w-full px-3 py-1.5 text-left text-xs transition-colors {item.disabled
          ? 'cursor-not-allowed text-muted/50'
          : 'hover:bg-accent-soft'} {item.danger && !item.disabled
          ? 'text-danger'
          : item.disabled
            ? ''
            : 'text-fg'}"
        role="menuitem"
        aria-disabled={item.disabled || undefined}
        onclick={() => contextMenu.run(item)}
      >
        {item.label}
      </button>
    {/each}
  </div>
{/if}
