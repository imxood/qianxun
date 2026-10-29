<script lang="ts">
  import type { Snippet } from 'svelte';

  /**
   * 设置行：左侧「标题 + 说明」，右侧控件。设置卡片的统一行形态；
   * 行距与分割线由卡片容器（divide-y）负责，行自身只管横向布局。
   */
  let {
    label,
    hint = '',
    warn = '',
    error = '',
    /** 控件占满剩余宽度（长输入框用）。 */
    fill = false,
    children,
  }: {
    label: string;
    hint?: string;
    warn?: string;
    error?: string;
    fill?: boolean;
    children?: Snippet;
  } = $props();
</script>

<div class="flex items-center justify-between gap-4 py-3 first:pt-1 last:pb-0">
  <div class={fill ? 'shrink-0' : 'min-w-0'}>
    <p class="text-sm leading-6 text-fg">{label}</p>
    {#if hint}<p class="text-xs leading-5 text-muted">{hint}</p>{/if}
    {#if warn}<p class="text-xs leading-5 text-warning">{warn}</p>{/if}
    {#if error}<p class="text-xs leading-5 text-danger">{error}</p>{/if}
  </div>
  {#if children}
    <div
      class={fill
        ? 'flex min-w-0 flex-1 items-center justify-end gap-2'
        : 'flex shrink-0 items-center gap-2'}
    >
      {@render children()}
    </div>
  {/if}
</div>
