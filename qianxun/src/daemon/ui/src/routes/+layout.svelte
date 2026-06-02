<script lang="ts">
	// Stage 7a +layout.svelte
	// - Sidebar + TopBar 框架
	// - mode-watcher 集成 (Stage 7b 完整 UI, 现在只 init)
	// - authStore 初始化 + token 弹窗 (401 或首次访问触发)
	// - 主题初始化

	import '../app.css';
	import { onMount } from 'svelte';
	import { ModeWatcher } from 'mode-watcher';
	import Sidebar from '$lib/components/layout/Sidebar.svelte';
	import TopBar from '$lib/components/layout/TopBar.svelte';
	import TokenDialog from '$lib/components/auth/TokenDialog.svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { themeStore } from '$lib/stores/theme.svelte';

	let { children } = $props();

	let tokenDialogOpen = $state(false);

	function onAuthFailed() {
		tokenDialogOpen = true;
	}

	onMount(() => {
		authStore.init();
		themeStore.init();

		// 没 token 且已初始化 → 弹框
		if (!authStore.isAuthenticated) {
			tokenDialogOpen = true;
		}

		window.addEventListener('qianxun:auth:failed', onAuthFailed);
		return () => {
			window.removeEventListener('qianxun:auth:failed', onAuthFailed);
		};
	});
</script>

<ModeWatcher defaultMode="system" />

<div class="bg-background flex h-screen w-screen overflow-hidden">
	<Sidebar />
	<div class="flex min-w-0 flex-1 flex-col">
		<TopBar onConfigureToken={() => (tokenDialogOpen = true)} />
		<main class="flex-1 overflow-auto p-6" data-testid="layout-main">
			{@render children?.()}
		</main>
	</div>
</div>

<TokenDialog open={tokenDialogOpen} onOpenChange={(v) => (tokenDialogOpen = v)} />
