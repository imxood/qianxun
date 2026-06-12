<script lang="ts">
	import Icon from '../shared/Icon.svelte';
	import StatusDot from '../shared/StatusDot.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { settingsStore } from '$lib/stores/settings.svelte';

	// 2026-06-09 加: Provider 名称动态显示 (跟 settingsStore.activeProvider)
	// 旧版硬编码 "DeepSeek", 现在从 store 读
	const providerLabel = $derived.by(() => {
		const id = settingsStore.activeProvider;
		if (id === 'deepseek') return 'DeepSeek';
		if (id === 'minimax') return 'minimax';
		// 自定义 provider: 首字母大写
		return id.charAt(0).toUpperCase() + id.slice(1);
	});
</script>

<div class="p-2 border-t border-zinc-200 dark:border-zinc-800 space-y-0.5">
	<button
		class="w-full flex items-center gap-2 px-2 py-1.5 text-xs text-zinc-600 dark:text-zinc-400 hover:bg-zinc-200/50 dark:hover:bg-zinc-800 rounded"
		onclick={() => uiStore.openSettings()}
		title="点击修改 Provider"
	>
		<Icon name="zap" class="w-3.5 h-3.5 text-amber-500" />
		<span>Provider · {providerLabel}</span>
	</button>
	<button
		class="w-full flex items-center gap-2 px-2 py-1.5 text-xs text-zinc-600 dark:text-zinc-400 hover:bg-zinc-200/50 dark:hover:bg-zinc-800 rounded"
		onclick={() => uiStore.openSettings()}
	>
		<Icon name="settings" class="w-3.5 h-3.5" />
		<span>设置</span>
	</button>
	<div class="flex items-center gap-2 px-2 py-1.5 text-xs">
		<StatusDot color="sky" />
		<span class="text-zinc-500">daemon 已连接</span>
		<div class="flex-1"></div>
		<button
			class="flex items-center gap-1.5 px-1.5 py-1 rounded text-zinc-500 dark:text-zinc-400 hover:bg-zinc-200/50 dark:hover:bg-zinc-800 hover:text-zinc-700 dark:hover:text-zinc-300"
			onclick={() => uiStore.toggleTheme()}
			aria-label="切换主题"
			title={uiStore.theme === 'dark' ? '切换到亮色' : '切换到暗色'}
		>
			<Icon name={uiStore.theme === 'dark' ? 'sun' : 'moon'} class="w-3.5 h-3.5" />
			<span>{uiStore.theme === 'dark' ? '亮色' : '暗色'}</span>
		</button>
	</div>
</div>
