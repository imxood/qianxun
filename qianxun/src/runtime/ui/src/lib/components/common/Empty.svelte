<script lang="ts">
	import { Inbox } from '@lucide/svelte';

	type Props = {
		title?: string;
		description?: string;
		icon?: typeof Inbox;
		class?: string;
	};

	let {
		title = '暂无数据',
		description = '',
		icon: Icon = Inbox,
		class: className = ''
	}: Props = $props();
</script>

<div
	class="border-border bg-card text-muted-foreground flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed p-12 text-center {className}"
	data-testid="empty-state"
>
	<Icon class="h-8 w-8 opacity-50" />
	<div class="text-sm font-medium">{title}</div>
	{#if description}
		<div class="text-xs">{description}</div>
	{/if}
</div>
