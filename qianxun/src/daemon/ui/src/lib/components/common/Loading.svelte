<script lang="ts">
	import { Loader2 } from '@lucide/svelte';

	type Props = {
		label?: string;
		class?: string;
	};

	let { label = '加载中…', class: className = '' }: Props = $props();
</script>

<div
	class="text-muted-foreground flex items-center justify-center gap-2 p-8 text-sm {className}"
	data-testid="loading-state"
	role="status"
	aria-live="polite"
>
	<Loader2 class="h-4 w-4 animate-spin" />
	<span>{label}</span>
</div>
