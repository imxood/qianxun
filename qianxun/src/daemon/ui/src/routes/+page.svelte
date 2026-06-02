<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';

	onMount(() => {
		void goto('/llm', { replaceState: true });
	});
</script>

<div class="text-muted-foreground flex items-center justify-center p-12 text-sm">
	跳转到 <a href="/llm" class="ml-1 underline">/llm</a> …
</div>
