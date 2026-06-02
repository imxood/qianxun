<script lang="ts" module>
	import { tv, type VariantProps } from 'tailwind-variants';

	export const tableVariants = tv({
		base: 'w-full caption-bottom text-sm',
		variants: {
			variant: {
				default: '',
				card: 'border-separate border-spacing-0'
			}
		},
		defaultVariants: { variant: 'default' }
	});
</script>

<script lang="ts">
	import type { HTMLAttributes } from 'svelte/elements';
	import { cn } from '$lib/utils';

	type Props = HTMLAttributes<HTMLTableElement> & {
		class?: string;
		children?: import('svelte').Snippet;
	};

	let { class: className = '', children, ...rest }: Props = $props();
</script>

<div class="relative w-full overflow-auto">
	<table class={cn(tableVariants(), className)} {...rest}>
		{@render children?.()}
	</table>
</div>
