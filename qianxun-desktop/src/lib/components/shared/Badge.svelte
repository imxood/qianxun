<script lang="ts">
	import Icon from './Icon.svelte';
	import type { PlanStatus, SubSessionStatus } from '$lib/types/entity';

	let { status }: { status: PlanStatus | SubSessionStatus | 'pending' } = $props();

	const labelMap: Record<string, string> = {
		pending: 'pending',
		running: 'running',
		done: 'done',
		failed: 'failed',
		aborted: 'aborted',
		readonly: 'read-only',
		active: 'running',
	};
	const label = $derived(labelMap[status] || status);
</script>

<span
	class="text-[10px] px-1.5 py-0.5 rounded font-medium"
	class:bg-sky-500={status === 'running' || status === 'active'}
	class:text-sky-100={status === 'running' || status === 'active'}
	class:bg-emerald-500={status === 'done'}
	class:text-emerald-100={status === 'done'}
	class:bg-rose-500={status === 'failed'}
	class:text-rose-100={status === 'failed'}
	class:bg-zinc-500={status === 'pending' || status === 'aborted' || status === 'readonly'}
	class:text-zinc-100={status === 'pending' || status === 'aborted' || status === 'readonly'}
>
	{label}
</span>
