<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
	import { projectStore } from '$lib/stores/project.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import type { Project, Session } from '$lib/types/entity';

	let { project, depth = 0 }: { project: Project; depth?: number } = $props();

	const isExpanded = $derived(uiStore.isProjectExpanded(project.id));
	const sessions = $derived(sessionStore.byProject(project.id));
	const activeId = $derived(uiStore.activeView.kind === 'session' ? uiStore.activeView.session_id : null);

	function statusOf(s: Session): 'running' | 'idle' {
		const runningPlan = planStore.bySession(s.id).find((p) => p.status === 'Running');
		return runningPlan ? 'running' : 'idle';
	}
</script>

<div>
	<button
		class="w-full flex items-center gap-1.5 px-2 py-1.5 text-[13px] hover:bg-zinc-200/50 dark:hover:bg-zinc-800 rounded"
		class:text-zinc-900={isExpanded}
		class:dark:text-zinc-100={isExpanded}
		class:text-zinc-700={!isExpanded}
		class:dark:text-zinc-300={!isExpanded}
		onclick={() => uiStore.toggleProjectExpanded(project.id)}
	>
		<Icon
			name={isExpanded ? 'chevron-down' : 'chevron-right'}
			class="w-3 h-3 text-zinc-500 flex-shrink-0"
		/>
		<Icon
			name={isExpanded ? 'folder-open' : 'folder'}
			class="w-3.5 h-3.5 flex-shrink-0 {isExpanded ? 'text-amber-500 dark:text-amber-400' : 'text-zinc-500'}"
		/>
		<span class="flex-1 text-left truncate">{project.name}</span>
		<span class="text-[11px] text-zinc-400">{sessions.length}</span>
	</button>

	{#if isExpanded && sessions.length > 0}
		<div
			class="ml-4 mt-0.5 space-y-0.5 border-l border-zinc-200 dark:border-zinc-800 pl-1.5"
		>
			{#each sessions as s (s.id)}
				{@const status = statusOf(s)}
				<button
					class={`w-full flex items-center gap-2 px-2 py-1.5 text-[12px] rounded ${activeId === s.id ? 'bg-zinc-200 dark:bg-zinc-800 text-zinc-900 dark:text-zinc-100' : 'text-zinc-600 dark:text-zinc-400 hover:bg-zinc-200/50 dark:hover:bg-zinc-800/30'}`}
					onclick={() => sessionStore.switchTo(s.id)}
				>
					<Icon
						name="message-square"
						class="w-3 h-3 flex-shrink-0 {status === 'running' ? (activeId === s.id ? 'text-amber-500' : 'text-amber-600 dark:text-amber-400') : 'text-zinc-400'}"
					/>
					<span class="flex-1 text-left truncate">{s.title}</span>
					{#if status === 'running'}
						<StatusDot color="sky" pulsing={true} />
					{/if}
				</button>
			{/each}
		</div>
	{/if}
</div>
