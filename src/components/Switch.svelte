<script lang="ts">
  interface Props {
    checked: boolean;
    onchange: (value: boolean) => void;
    label: string;
    disabled?: boolean;
  }

  let { checked, onchange, label, disabled = false }: Props = $props();
</script>

<button
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  class="relative h-5 w-9 shrink-0 rounded-full transition-colors {checked
    ? 'bg-accent'
    : 'bg-line'} disabled:cursor-not-allowed disabled:opacity-50"
  onclick={() => onchange(!checked)}
>
  <span
    class="absolute top-0.5 size-4 rounded-full bg-white shadow transition-all {checked
      ? 'left-[1.125rem]'
      : 'left-0.5'}"
  ></span>
</button>
