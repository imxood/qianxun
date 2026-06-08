<script lang="ts">
	import { uiStore } from '$lib/stores/ui.svelte';
</script>

<button
	class="w-1 bg-transparent hover:bg-amber-500/50 transition-colors relative cursor-col-resize flex-shrink-0 group"
	aria-label="拖动调整宽度"
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
></button>

<div class="w-px bg-zinc-200 dark:bg-zinc-800 flex-shrink-0"></div>
