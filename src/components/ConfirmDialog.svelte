<script lang="ts">
  /**
   * 通用确认对话框（模态）。替代 `window.confirm`——WebView2 下原生
   * confirm 返回值不可靠（可能直接返回假值），且无法定制样式与文案。
   * Esc = 取消，Enter = 确认；点遮罩等效取消。
   */
  let {
    open,
    title,
    message,
    confirmLabel = '确定',
    cancelLabel = '取消',
    danger = false,
    onconfirm,
    oncancel,
  }: {
    open: boolean;
    title: string;
    message: string;
    confirmLabel?: string;
    cancelLabel?: string;
    danger?: boolean;
    onconfirm?: () => void;
    oncancel?: () => void;
  } = $props();

  function onKeydown(event: KeyboardEvent): void {
    if (!open) return;
    if (event.key === 'Escape') oncancel?.();
    if (event.key === 'Enter') onconfirm?.();
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
    role="presentation"
    onclick={() => oncancel?.()}
  >
    <div
      class="w-80 rounded-lg border border-line bg-surface p-4 shadow-xl"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onclick={(event) => event.stopPropagation()}
    >
      <h3 class="text-sm font-medium text-fg">{title}</h3>
      <p class="mt-1.5 text-xs leading-5 text-muted">{message}</p>
      <div class="mt-4 flex justify-end gap-2">
        <button
          class="rounded-md px-3 py-1.5 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
          onclick={() => oncancel?.()}
        >
          {cancelLabel}
        </button>
        <button
          class="rounded-md px-3 py-1.5 text-xs font-medium text-white transition-colors {danger
            ? 'bg-danger hover:bg-danger/90'
            : 'bg-accent hover:bg-accent/90'}"
          onclick={() => onconfirm?.()}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}
