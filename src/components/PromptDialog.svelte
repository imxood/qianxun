<script lang="ts">
  /**
   * 通用单行输入对话框（模态）：替代 `window.prompt`（跨平台兼容性差、
   * 无法校验）。打开时重置为初始值并聚焦；空值不允许提交。
   */
  let {
    open,
    title,
    label,
    placeholder = '',
    initialValue = '',
    confirmLabel = '确定',
    cancelLabel = '取消',
    onconfirm,
    oncancel,
  }: {
    open: boolean;
    title: string;
    label: string;
    placeholder?: string;
    initialValue?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    onconfirm?: (value: string) => void;
    oncancel?: () => void;
  } = $props();

  let value = $state('');
  let input: HTMLInputElement | null = $state(null);

  // 每次打开重置并聚焦（rAF 等 DOM 就位）。
  $effect(() => {
    if (!open) return;
    value = initialValue;
    requestAnimationFrame(() => input?.focus());
  });

  function submit(): void {
    const trimmed = value.trim();
    if (!trimmed) return;
    onconfirm?.(trimmed);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (!open) return;
    if (event.key === 'Escape') oncancel?.();
    if (event.key === 'Enter') submit();
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
      <label class="mt-3 block text-xs text-muted">
        {label}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="mt-1 w-full rounded-md border border-line bg-bg px-2.5 py-1.5 text-xs text-fg outline-none focus:border-accent"
          bind:this={input}
          bind:value
          {placeholder}
          autocomplete="off"
          autofocus
        />
      </label>
      <div class="mt-4 flex justify-end gap-2">
        <button
          class="rounded-md px-3 py-1.5 text-xs text-muted transition-colors hover:bg-accent-soft hover:text-fg"
          onclick={() => oncancel?.()}
        >
          {cancelLabel}
        </button>
        <button
          class="rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-accent/90"
          onclick={submit}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  </div>
{/if}
