<script lang="ts">
	import type { HTMLAttributes } from 'svelte/elements';
	import { cn } from '$lib/utils';

	type Props = HTMLAttributes<HTMLTableSectionElement> & {
		class?: string;
		children?: import('svelte').Snippet;
	};

	let { class: className = '', children, ...rest }: Props = $props();
</script>

<thead class={cn('[&_tr]:border-b', className)} {...rest}>
	{@render children?.()}
</thead>
