<script lang="ts">
	import { AlertTriangle } from '@lucide/svelte';

	type Props = {
		message: string;
		class?: string;
	};

	let { message, class: className = '' }: Props = $props();
</script>

<div
	class="text-destructive border-destructive/30 bg-destructive/5 flex items-center gap-2 rounded-md border p-3 text-sm {className}"
	data-testid="error-state"
	role="alert"
>
	<AlertTriangle class="h-4 w-4 shrink-0" />
	<span>{message}</span>
</div>
