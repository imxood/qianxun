<script lang="ts">
	import Icon from '../shared/Icon.svelte';

	let {
		items = ['未选', '千寻桌面端', '千寻 daemon', 'qianxun-test', 'scratch'],
	} = $props();
</script>

<div class="mb-2 flex items-center gap-1.5 px-1 flex-wrap">
	<button
		class="text-[11px] px-2 py-0.5 rounded bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 hover:border-zinc-400 dark:hover:border-zinc-700 text-zinc-700 dark:text-zinc-300 flex items-center gap-1"
	>
		<Icon name="folder" class="w-3 h-3 text-amber-500" />
		<span>{items[0]}</span>
		<Icon name="chevron-down" class="w-2.5 h-2.5 text-zinc-500" />
	</button>
	<button
		class="text-[11px] px-2 py-0.5 rounded bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-800 hover:border-zinc-400 dark:hover:border-zinc-700 text-zinc-700 dark:text-zinc-300 flex items-center gap-1"
	>
		<Icon name="cpu" class="w-3 h-3 text-zinc-500" />
		<span>MiniMax-M3</span>
	</button>
	<span class="text-[10px] text-zinc-500 dark:text-zinc-600 ml-1">选文件夹 → 自动归类到项目</span>
</div>
