<script lang="ts">
  import { contextMenu } from '../lib/menu.svelte';

  // 点击别处 / Esc / 滚动都收起菜单；右键其他目标由 show() 先 close 再开新。
  function dismiss(): void {
    contextMenu.close();
  }
</script>

<svelte:window
  onclick={dismiss}
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
        class="block w-full px-3 py-1.5 text-left text-xs transition-colors hover:bg-accent-soft {item.danger
          ? 'text-danger'
          : 'text-fg'}"
        role="menuitem"
        onclick={() => contextMenu.run(item)}
      >
        {item.label}
      </button>
    {/each}
  </div>
{/if}
