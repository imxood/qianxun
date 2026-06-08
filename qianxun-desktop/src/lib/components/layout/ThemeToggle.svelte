<script lang="ts">
	import { uiStore } from '$lib/stores/ui.svelte';
	import Icon from '../shared/Icon.svelte';
</script>

<button
	class="p-1.5 rounded hover:bg-zinc-200 dark:hover:bg-zinc-800 text-zinc-500 dark:text-zinc-400"
	onclick={() => uiStore.toggleTheme()}
	aria-label="切换主题"
	title={uiStore.theme === 'dark' ? '切换到亮色' : '切换到暗色'}
>
	<Icon name={uiStore.theme === 'dark' ? 'sun' : 'moon'} class="w-4 h-4" />
</button>
