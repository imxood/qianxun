<script lang="ts">
	import { onMount } from 'svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import Empty from '$lib/components/common/Empty.svelte';
	import { getStatus } from '$lib/api/system';

	let version = $state<string>('?');
	let stage = $state<string>('?');

	onMount(async () => {
		try {
			const r = await getStatus();
			version = r.version;
			stage = r.stage;
		} catch {
			/* ignore */
		}
	});
</script>

<PageHeader
	title="System"
	description="Daemon 健康 / 资源 / 日志 (Stage 7b 完整)"
/>
<Empty
	title="Stage 7a 暂未开放"
	description="系统监控面板 (CPU/内存/连接数/日志) 在 Stage 7b 实现. 当前 daemon: v{version} ({stage})"
/>
