<script lang="ts">
	import { onMount } from 'svelte';
	import ThreeColumnLayout from '$lib/components/layout/ThreeColumnLayout.svelte';
	import TopBar from '$lib/components/layout/TopBar.svelte';
	import Sidebar from '$lib/components/col1/Sidebar.svelte';
	import ChatView from '$lib/components/col2/ChatView.svelte';
	import Inspector from '$lib/components/col3/Inspector.svelte';
	import ExperienceSuggestModal from '$lib/components/modals/ExperienceSuggestModal.svelte';
	import SettingsModal from '$lib/components/modals/SettingsModal.svelte';
	import LoadingSplash from '$lib/components/layout/LoadingSplash.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { bootOnce, type BootState } from '$lib/boot';

	// 2026-06-09: 启动 splash 屏状态. 2026-06-12 抽到 lib/boot.ts, 组件只关心状态绑定.
	const bootState: BootState = $state({ step: 'starting', progress: 0, error: null });
	let bootstrapped = $state(false);

	async function runBoot() {
		// 2026-06-09: 删掉 app.html 里的静态 splash (CSS-only).
		const initSplash = document.getElementById('__init_splash');
		if (initSplash) initSplash.remove();
		await bootOnce(bootState);
		// 2026-06-09: Tauri 官方 splash 方案. 通知后端前端 ready, 配合 backend ready 关 splash + 显示 main.
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			await invoke('set_complete', { task: 'frontend' });
		} catch {
			// web dev 模式无后端, 忽略
		}
		// 短延迟让用户看到 "100% 就绪" 一瞬, 然后隐藏 splash
		setTimeout(() => {
			bootstrapped = true;
		}, 300);
	}

	function bootRetry() {
		bootstrapped = false;
		bootState.error = null;
		runBoot();
	}

	onMount(() => {
		runBoot();
	});
</script>

<!-- 2026-06-09: 启动 splash. 2026-06-12 错误用 banner 持续显示, 用户可点 "重试" -->
{#if !bootstrapped}
	<LoadingSplash
		step={bootState.step}
		progress={bootState.progress}
		error={bootState.error}
		onRetry={bootRetry}
	/>
{:else}
	<ThreeColumnLayout>
		{#snippet sidebar()}
			<Sidebar />
		{/snippet}

		<TopBar />
		<ChatView />

		{#snippet sessions()}
			<Inspector />
		{/snippet}
	</ThreeColumnLayout>
{/if}

<!-- 暂未启用 (Stage 4a 之前的 mock 演示保留, 等真实 experience 沉淀功能再恢复) -->
<!-- 2026-06-09 清掉 mock: 移除 1.5s/8s setTimeout, splash 屏 + session 列表才是真正的"已连接"反馈 -->
<!--
<ExperienceSuggestModal
	open={showExperienceModal}
	onClose={() => (showExperienceModal = false)}
	items={[
		{ content: '本项目用 jose 库做 JWT, 不用 jsonwebtoken (TypeScript 友好)' },
		{ content: 'bcrypt rounds=12, 平衡性能跟安全' },
		{ content: 'JWT 用 RS256 非对称签名, 公钥可下发到多端' },
	]}
/>
-->

<!-- 2026-06-09 加: Settings 模态 (Provider / API key / Model) -->
<SettingsModal
	open={uiStore.settingsModalOpen}
	onClose={() => uiStore.closeSettings()}
/>
