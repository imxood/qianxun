<script lang="ts">
	let { color = 'sky', pulsing = false, size = 'w-1.5 h-1.5' }: { color?: 'sky' | 'emerald' | 'rose' | 'amber' | 'zinc'; pulsing?: boolean; size?: string } = $props();
</script>

<span
	class="inline-block rounded-full {size} {pulsing ? 'animate-pulse' : ''}"
	class:bg-sky-400={color === 'sky'}
	class:bg-emerald-400={color === 'emerald'}
	class:bg-rose-400={color === 'rose'}
	class:bg-amber-400={color === 'amber'}
	class:bg-zinc-400={color === 'zinc'}
></span>
