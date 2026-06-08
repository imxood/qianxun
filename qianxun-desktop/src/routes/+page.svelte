<script lang="ts">
	import { onMount } from 'svelte';
	import { browser } from '$app/environment';
	import ThreeColumnLayout from '$lib/components/layout/ThreeColumnLayout.svelte';
	import TopBar from '$lib/components/layout/TopBar.svelte';
	import Sidebar from '$lib/components/col1/Sidebar.svelte';
	import ChatView from '$lib/components/col2/ChatView.svelte';
	import Inspector from '$lib/components/col3/Inspector.svelte';
	import ExperienceSuggestModal from '$lib/components/modals/ExperienceSuggestModal.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { sessionStore } from '$lib/stores/session.svelte';

	let showExperienceModal = $state(false);

	onMount(() => {
		// 监听 ⌘N 新建
		function onKeydown(e: KeyboardEvent) {
			if ((e.metaKey || e.ctrlKey) && e.key === 'n') {
				e.preventDefault();
				const projectId = uiStore.activeView.kind === 'session'
					? sessionStore.active?.project_id ?? null
					: null;
				uiStore.switchToNew(projectId);
			}
		}
		if (browser) {
			window.addEventListener('keydown', onKeydown);

			// 演示: 5s 后弹一个"经验沉淀" toast (展示 toast 组件)
			setTimeout(() => {
				if (browser) {
					uiStore.pushToast({
						kind: 'success',
						title: '已连接到 daemon',
						description: '127.0.0.1:23900 · 5 实体已加载',
						timeout_ms: 4000,
					});
				}
			}, 1500);

			setTimeout(() => {
				showExperienceModal = true;
			}, 8000);
		}
		return () => {
			if (browser) {
				window.removeEventListener('keydown', onKeydown);
			}
		};
	});
</script>

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

<ExperienceSuggestModal
	open={showExperienceModal}
	onClose={() => (showExperienceModal = false)}
	items={[
		{ content: '本项目用 jose 库做 JWT, 不用 jsonwebtoken (TypeScript 友好)' },
		{ content: 'bcrypt rounds=12, 平衡性能跟安全' },
		{ content: 'JWT 用 RS256 非对称签名, 公钥可下发到多端' },
	]}
/>
