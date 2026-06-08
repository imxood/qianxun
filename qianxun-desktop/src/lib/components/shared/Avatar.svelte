<script lang="ts">
	import Icon from './Icon.svelte';

	let { kind = 'user', size = 'w-7 h-7' }: { kind?: 'user' | 'assistant' | 'plan' | 'bot'; size?: string } = $props();
</script>

{#if kind === 'user'}
	<div class="{size} rounded-full bg-zinc-200 dark:bg-zinc-700 flex items-center justify-center text-zinc-700 dark:text-zinc-200 flex-shrink-0">
		<Icon name="user" class="w-3.5 h-3.5" />
	</div>
{:else if kind === 'assistant'}
	<div class="{size} rounded-full bg-gradient-to-br from-amber-400 to-amber-600 flex items-center justify-center text-zinc-950 flex-shrink-0 shadow">
		<Icon name="sparkles" class="w-3.5 h-3.5" />
	</div>
{:else if kind === 'plan'}
	<div class="{size} rounded-md bg-zinc-200 dark:bg-zinc-800 flex items-center justify-center text-zinc-500 flex-shrink-0">
		<Icon name="layers" class="w-3.5 h-3.5" />
	</div>
{:else}
	<div class="{size} rounded-full bg-sky-900/50 flex items-center justify-center flex-shrink-0">
		<Icon name="bot" class="w-3.5 h-3.5 text-sky-400" />
	</div>
{/if}
