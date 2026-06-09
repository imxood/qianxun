<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
	import { projectStore } from '$lib/stores/project.svelte';
	import { seedStore } from '$lib/stores/seed.svelte';

	let query = $state('');

	async function onNewTask() {
		const project = projectStore.get('proj_qianxun_desktop') ?? null;
		// 2026-06-09 lazy create: 只切到 'new' view, 不调 create_session invoke.
		// 真正的 session 持久化等用户发送第一条消息时 (chatStore.send) 才发生.
		// 理由: 避免"点新建任务但永远不发消息"产生的空 session 浪费 SQLite row.
		uiStore.switchToNew(project?.id ?? null);
	}
</script>

<div class="p-2 space-y-1.5 border-b border-zinc-200 dark:border-zinc-800">
	<button
		class="w-full flex items-center gap-2 px-2 py-1.5 text-zinc-700 dark:text-zinc-200 hover:bg-zinc-200/50 dark:hover:bg-zinc-800 rounded text-[13px]"
		onclick={onNewTask}
	>
		<Icon name="plus-circle" class="w-4 h-4 text-zinc-500 dark:text-zinc-400" />
		<span>新建任务</span>
		<span class="ml-auto text-[10px] text-zinc-400">⌘N</span>
	</button>
	<div class="relative">
		<Icon name="search" class="w-3.5 h-3.5 text-zinc-400 absolute left-2.5 top-1/2 -translate-y-1/2" />
		<input
			type="text"
			placeholder="搜索任务 / 文件 / 命令"
			bind:value={query}
			class="w-full pl-8 pr-2 py-1.5 bg-white dark:bg-zinc-950 border border-zinc-200 dark:border-zinc-800 rounded-md text-xs text-zinc-900 dark:text-zinc-200 placeholder-zinc-400 focus:border-zinc-400 dark:focus:border-zinc-700 focus:outline-none"
		/>
	</div>
</div>
