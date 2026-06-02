<script lang="ts">
	type Props = {
		title: string;
		description?: string;
		actions?: import('svelte').Snippet;
		children?: import('svelte').Snippet;
	};

	let { title, description, actions, children }: Props = $props();
</script>

<header class="mb-4 flex items-start justify-between gap-3">
	<div class="flex flex-col gap-1">
		<h1 class="text-xl font-semibold tracking-tight">{title}</h1>
		{#if description}
			<p class="text-muted-foreground text-sm">{description}</p>
		{/if}
	</div>
	{#if actions}
		<div class="flex items-center gap-2">{@render actions()}</div>
	{/if}
</header>
{#if children}
	<section>{@render children()}</section>
{/if}
