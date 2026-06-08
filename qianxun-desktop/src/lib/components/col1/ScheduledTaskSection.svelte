<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
	import { buildSeed } from '$lib/mock/seed';

	const tasks = $derived(buildSeed().scheduled);
</script>

<div>
	<div class="px-3 py-1 text-[11px] text-zinc-500 font-medium tracking-wide">定时任务</div>
	<div class="px-1.5 space-y-0.5">
		{#each tasks as task (task.id)}
			<button
				class="w-full flex items-center gap-2 px-2 py-1.5 text-[13px] text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200/50 dark:hover:bg-zinc-800 rounded"
				title={task.cron}
			>
				<Icon name="folder" class="w-3.5 h-3.5 text-zinc-500 flex-shrink-0" />
				<span class="flex-1 text-left truncate">{task.name}</span>
				<StatusDot color="sky" pulsing={true} />
			</button>
		{/each}
	</div>
</div>
