<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import PlanSummary from './PlanSummary.svelte';
	import TaskList from './TaskList.svelte';
	import ChangedFiles from './ChangedFiles.svelte';
	import NewSessionHints from './NewSessionHints.svelte';
	import SubSessionContext from './SubSessionContext.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
</script>

<aside class="h-full flex flex-col bg-zinc-50 dark:bg-zinc-900 text-zinc-900 dark:text-zinc-100">
	<div class="px-3 py-2 border-b border-zinc-200 dark:border-zinc-800 flex items-center justify-between">
		<h5 class="text-[11px] font-semibold text-zinc-500 uppercase tracking-wider">Inspector</h5>
		<div class="flex items-center gap-1">
			<button class="p-1 rounded hover:bg-zinc-200/50 dark:hover:bg-zinc-800 text-zinc-500" aria-label="刷新">
				<Icon name="refresh-cw" class="w-3.5 h-3.5" />
			</button>
			<button
				class="p-1 rounded hover:bg-zinc-200/50 dark:hover:bg-zinc-800 text-zinc-500"
				aria-label="收起 Inspector"
				title="收起 Inspector"
				onclick={() => uiStore.toggleCol3Collapsed()}
			>
				<Icon name="panel-right-close" class="w-3.5 h-3.5" />
			</button>
		</div>
	</div>
	<div class="flex-1 overflow-y-auto">
		{#if uiStore.activeView.kind === 'sub_session'}
			<SubSessionContext />
		{:else if planStore.active}
			<PlanSummary />
			<TaskList plan={planStore.active} />
			<ChangedFiles plan={planStore.active} />
		{:else if sessionStore.active}
			<NewSessionHints />
		{:else}
			<NewSessionHints />
		{/if}
	</div>
</aside>
