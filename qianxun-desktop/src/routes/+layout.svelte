<script lang="ts">
	import '../routes/layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import Toast from '$lib/components/shared/Toast.svelte';

	let { children } = $props();
</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>千寻 (Qianxun)</title>
</svelte:head>

<div class="h-screen">
	{@render children?.()}
</div>

<Toast />
