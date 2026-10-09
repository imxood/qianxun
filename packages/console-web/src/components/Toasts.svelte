<script lang="ts">
  import { dismiss, toasts } from '../lib/toast.svelte';
</script>

<!-- 右下角固定位（docs/20 §4.10）：成功可撤销、失败带原因，5 秒自动收起。 -->
<div class="pointer-events-none fixed right-5 bottom-5 z-50 flex flex-col items-end gap-2">
  {#each toasts.items as item (item.id)}
    <div
      class="pointer-events-auto flex items-center gap-3 rounded-xl border px-3.5 py-2.5 text-[13px] shadow-[0_8px_24px_rgba(0,0,0,.45)] {item.kind ===
      'fail'
        ? 'border-danger/45 bg-[#2a1414] text-danger'
        : 'border-line-strong bg-surface2 text-fg'}"
      role="status"
    >
      <span>{item.text}</span>
      {#if item.undo}
        <button
          class="rounded-md px-1.5 py-0.5 text-[13px] text-accent hover:bg-surface"
          onclick={() => {
            item.undo?.();
            dismiss(item.id);
          }}>撤销</button
        >
      {/if}
    </div>
  {/each}
</div>
