<script lang="ts">
	import { uiStore } from '$lib/stores/ui.svelte';

	let { children, sidebar, sessions }: { children?: any; sidebar?: any; sessions?: any } = $props();

	const col1 = $derived(uiStore.col1Width);
	const col3 = $derived(uiStore.col3Width);
</script>

<div class="flex h-screen w-screen overflow-hidden bg-zinc-50 dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100">
	{#if sidebar}
		<aside class="flex-shrink-0" style:width="{col1}px">
			{@render sidebar?.()}
		</aside>
		<div class="w-1 cursor-col-resize bg-zinc-200 dark:bg-zinc-800 hover:bg-amber-500/50 transition-colors flex-shrink-0"
			onmousedown={(e) => {
				const startX = e.clientX;
				const startW = uiStore.col1Width;
				const onMove = (ev: MouseEvent) => {
					const dx = ev.clientX - startX;
					uiStore.setCol1Width(startW + dx);
				};
				const onUp = () => {
					document.removeEventListener('mousemove', onMove);
					document.removeEventListener('mouseup', onUp);
					document.body.style.cursor = '';
				};
				document.addEventListener('mousemove', onMove);
				document.addEventListener('mouseup', onUp);
				document.body.style.cursor = 'col-resize';
				e.preventDefault();
			}}
		></div>
	{/if}

	<main class="flex-1 min-w-0 flex flex-col">
		{@render children?.()}
	</main>

	{#if !uiStore.col3Collapsed && sessions}
		<div class="w-1 cursor-col-resize bg-zinc-200 dark:bg-zinc-800 hover:bg-amber-500/50 transition-colors flex-shrink-0"
			onmousedown={(e) => {
				const startX = e.clientX;
				const startW = uiStore.col3Width;
				const onMove = (ev: MouseEvent) => {
					const dx = ev.clientX - startX;
					uiStore.setCol3Width(startW - dx);
				};
				const onUp = () => {
					document.removeEventListener('mousemove', onMove);
					document.removeEventListener('mouseup', onUp);
					document.body.style.cursor = '';
				};
				document.addEventListener('mousemove', onMove);
				document.addEventListener('mouseup', onUp);
				document.body.style.cursor = 'col-resize';
				e.preventDefault();
			}}
		></div>
		<aside class="flex-shrink-0" style:width="{col3}px">
			{@render sessions?.()}
		</aside>
	{/if}
</div>
