<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
</script>

<div>
	<div class="px-3 py-1 text-[11px] text-zinc-500 font-medium tracking-wide flex items-center justify-between">
		<span>Agent 团队</span>
		<button class="text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300" aria-label="新建团队">
			<Icon name="plus" class="w-3 h-3" />
		</button>
	</div>
	<div class="px-3 py-2 text-[11px] text-zinc-500">无团队, 点 + 新建</div>
</div>
