<script lang="ts">
  /**
   * 全局 toast 浮层（汇总 §3.16）：单例 toast 队列 → 底部居中显示。
   * 用 search domain locate.ts 等原本静默吞错的入口。
   */
  import { toast } from '../stores/toast.svelte';
</script>

{#if toast.current}
  <div
    class="pointer-events-none fixed inset-x-0 bottom-12 z-50 flex justify-center px-4"
    aria-live="polite"
  >
    <div
      class="pointer-events-auto flex max-w-md items-center gap-3 rounded-lg border border-line bg-surface px-4 py-2 text-sm shadow-lg"
      role="status"
    >
      <span class="min-w-0 flex-1 truncate">{toast.current.text}</span>
      {#if toast.current.action}
        <button
          class="shrink-0 rounded px-2 py-0.5 text-xs font-medium text-accent hover:bg-accent-soft"
          onclick={() => {
            const action = toast.current?.action;
            toast.dismiss();
            action?.run();
          }}
        >
          {toast.current.action.label}
        </button>
      {/if}
      <button
        class="shrink-0 text-muted hover:text-fg"
        aria-label="关闭通知"
        onclick={() => toast.dismiss()}
      >
        ×
      </button>
    </div>
  </div>
{/if}
