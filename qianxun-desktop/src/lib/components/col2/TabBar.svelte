<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
	import { subSessionStore } from '$lib/stores/sub_session.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import type { SubSession } from '$lib/types/entity';

	let { planId }: { planId: string } = $props();
	const subs = $derived(subSessionStore.byPlan(planId));
	const parent = $derived(subs[0] ? sessionStore.get(subs[0].parent_session_id) : null);

	function isActive(sub: SubSession): boolean {
		return uiStore.activeView.kind === 'sub_session' && uiStore.activeView.sub_session_id === sub.id;
	}
</script>

<div class="h-9 px-2 border-b border-zinc-200 dark:border-zinc-800 flex items-center gap-1 flex-shrink-0 bg-white dark:bg-zinc-950">
	<button
		class="h-7 px-3 flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-400 hover:text-zinc-900 dark:hover:text-zinc-200 rounded-t-md"
		onclick={() => parent && sessionStore.switchTo(parent.id)}
	>
		<Icon name="message-square" class="w-3.5 h-3.5" />
		<span>主会话{parent ? `: ${parent.title.slice(0, 16)}` : ''}</span>
		<Icon name="x" class="w-3 h-3 opacity-50 hover:opacity-100" />
	</button>
	{#each subs as sub (sub.id)}
		<button
			class="h-7 px-3 flex items-center gap-2 text-xs rounded-t-md"
			class:bg-zinc-100={isActive(sub)}
			class:dark:bg-zinc-900={isActive(sub)}
			class:text-zinc-900={isActive(sub)}
			class:dark:text-zinc-100={isActive(sub)}
			class:text-zinc-600={!isActive(sub)}
			class:dark:text-zinc-400={!isActive(sub)}
			onclick={() => uiStore.switchToSubSession(sub.id, sub.parent_session_id)}
		>
			<Icon name="bot" class="w-3.5 h-3.5 text-sky-500" />
			<span>子会话: {sub.role}</span>
			{#if sub.status === 'Active'}
				<StatusDot color="sky" pulsing={true} />
			{/if}
			<Icon name="x" class="w-3 h-3 opacity-50 hover:opacity-100" />
		</button>
	{/each}
</div>
