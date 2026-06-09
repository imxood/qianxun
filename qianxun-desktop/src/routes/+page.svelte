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
	import { sessionStore } from '$lib/stores/session.svelte';
	import { chatStore } from '$lib/stores/chat.svelte';
	import { projectStore } from '$lib/stores/project.svelte';

	// 2026-06-09 加: 启动 splash 屏状态 (直到 3 步 init 完成才显示主布局).
	let bootstrapped = $state(false);
	let bootStep = $state<'starting' | 'connecting' | 'loading' | 'ready'>('starting');
	let bootProgress = $state(0);
	// 2026-06-09 体验打磨: 启动错误用 banner 持续显示, 不一闪而过
	let bootError = $state<string | null>(null);

	async function bootOnce() {
		// 2026-06-09: 启动 timing 诊断. 配合 Tauri 端 [boot] T0..T2 日志, 精确定位哪一跳慢.
		// 输出格式: "[boot F0.1] chatStore.init: 1234ms" (F 表示 frontend, T 表示 tauri)
		const tBoot = performance.now();
		const tMark = (label: string) => {
			console.log(`[boot F${label}] +${Math.round(performance.now() - tBoot)}ms`);
		};

		// 2026-06-09: 删掉 app.html 里的静态 splash (CSS-only).
		// Svelte LoadingSplash 组件接管. 避免两个 splash 叠加.
		const initSplash = document.getElementById('__init_splash');
		if (initSplash) initSplash.remove();
		tMark('0.0 bootOnce entered (init_splash removed)');

		bootError = null;
		bootStep = 'starting';
		bootProgress = 20;

		// 步骤 1: session_event listener 注册
		try {
			const t1 = performance.now();
			await chatStore.init();
			tMark(`1.0 chatStore.init done (${Math.round(performance.now() - t1)}ms)`);
		} catch (e) {
			console.error('[boot] chatStore.init failed', e);
			bootError = `事件监听器注册失败: ${String(e)}。可继续使用, 但实时事件可能不工作。`;
		}

		// 步骤 2: 并行拉 session + project 列表
		try {
			bootStep = 'connecting';
			bootProgress = 50;
			const t2 = performance.now();
			await Promise.all([sessionStore.init(), projectStore.loadAll()]);
			tMark(`2.0 session+project init done (${Math.round(performance.now() - t2)}ms)`);
		} catch (e) {
			console.error('[boot] session/project init failed', e);
			bootError = (bootError ? bootError + '\n' : '') + `无法加载会话/项目列表: ${String(e)}`;
		}

		// 步骤 3: 切到 session view
		bootStep = 'loading';
		bootProgress = 80;
		const firstSession = sessionStore.all[0];
		if (firstSession) {
			uiStore.switchToSession(firstSession.id);
		}
		tMark('3.0 first session switch done');

		// 完成
		bootStep = 'ready';
		bootProgress = 100;

		// 2026-06-09: Tauri 官方 splash 方案. 通知后端前端 ready, 配合 backend ready 关 splash + 显示 main.
		// 必须在切换主布局前调, 否则用户看到主布局瞬间但 splash 还盖在上面.
		try {
			const { invoke } = await import('@tauri-apps/api/core');
			await invoke('set_complete', { task: 'frontend' });
			tMark('3.5 set_complete(frontend) invoked');
		} catch (e) {
			console.warn('[boot] set_complete(frontend) failed (web dev mode ok):', e);
		}

		// 短延迟让用户看到 "100% 就绪" 一瞬, 然后隐藏 splash
		setTimeout(() => {
			bootstrapped = true;
			tMark('4.0 splash hidden, main layout shown');
		}, 300);
	}

	function bootRetry() {
		bootstrapped = false;
		bootOnce();
	}

	onMount(() => {
		bootOnce();
	});
</script>

<!-- 2026-06-09 加: 启动 splash, 直到 onMount 完成 + 切到 session 视图才显示主布局.
     解决 "tauri 启动 10+ 秒空白" UX 问题.
     2026-06-09 体验打磨: 错误用顶部 banner 持续显示, 用户可点 "重试" -->
{#if !bootstrapped}
	<LoadingSplash
		step={bootStep}
		progress={bootProgress}
		error={bootError}
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
