<script lang="ts">
	import Modal from '../shared/Modal.svelte';
	import { settingsStore } from '$lib/stores/settings.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';

	let { open, onClose }: { open: boolean; onClose: () => void } = $props();

	// 预设 provider 列表 (跟后端 config.rs::default_*_for 配对)
	const PRESETS = [
		{ id: 'deepseek', label: 'DeepSeek', defaultModel: 'deepseek-v4-flash', defaultUrl: 'https://api.deepseek.com/anthropic' },
		{ id: 'minimax', label: 'minimax', defaultModel: 'MiniMax-M3', defaultUrl: 'https://api.minimaxi.com/anthropic' },
	] as const;

	// 局部表单状态 (输入未保存)
	let activeProvider = $state(settingsStore.activeProvider);
	let apiKey = $state('');
	let model = $state('');
	let baseUrl = $state('');

	// 打开时同步
	$effect(() => {
		if (open) {
			activeProvider = settingsStore.activeProvider;
			// 不回填 apiKey (敏感, 永远让用户重输)
			apiKey = '';
			model = '';
			baseUrl = '';
		}
	});

	async function onSave() {
		const provider = PRESETS.find((p) => p.id === activeProvider);
		const config = {
			...(apiKey ? { api_key: apiKey } : {}),
			...(model ? { model } : provider ? { model: provider.defaultModel } : {}),
			...(baseUrl ? { base_url: baseUrl } : provider ? { base_url: provider.defaultUrl } : {}),
		};
		await settingsStore.setActiveProvider(activeProvider, config);
		onClose();
	}

	function onReset() {
		settingsStore.reset();
		onClose();
		uiStore.pushToast({ kind: 'info', title: '设置已重置', timeout_ms: 2000 });
	}
</script>

<Modal {open} {onClose} title="设置" maxWidth="max-w-xl">
	<div class="space-y-4 text-sm">
		<!-- Provider 选择 -->
		<div>
			<label class="block text-xs font-medium text-zinc-700 dark:text-zinc-300 mb-1">
				激活的 Provider
			</label>
			<select
				bind:value={activeProvider}
				class="w-full px-3 py-2 rounded border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100"
			>
				{#each PRESETS as p}
					<option value={p.id}>{p.label}</option>
				{/each}
			</select>
		</div>

		<!-- Model -->
		<div>
			<label class="block text-xs font-medium text-zinc-700 dark:text-zinc-300 mb-1">
				Model (留空用默认)
			</label>
			<input
				type="text"
				bind:value={model}
				placeholder={PRESETS.find((p) => p.id === activeProvider)?.defaultModel ?? ''}
				class="w-full px-3 py-2 rounded border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100 placeholder-zinc-400"
			/>
		</div>

		<!-- Base URL -->
		<div>
			<label class="block text-xs font-medium text-zinc-700 dark:text-zinc-300 mb-1">
				Base URL (留空用默认)
			</label>
			<input
				type="text"
				bind:value={baseUrl}
				placeholder={PRESETS.find((p) => p.id === activeProvider)?.defaultUrl ?? ''}
				class="w-full px-3 py-2 rounded border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100 placeholder-zinc-400"
			/>
		</div>

		<!-- API Key -->
		<div>
			<label class="block text-xs font-medium text-zinc-700 dark:text-zinc-300 mb-1">
				API Key (留空不修改, 优先 env 变量)
			</label>
			<input
				type="password"
				bind:value={apiKey}
				placeholder="sk-..."
				class="w-full px-3 py-2 rounded border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-950 text-zinc-900 dark:text-zinc-100 placeholder-zinc-400"
			/>
			<p class="mt-1 text-[11px] text-zinc-500">
				API key 优先级: 此处 → env 变量 (<code>DEEPSEEK_API_KEY</code> 等) → 配置文件
			</p>
		</div>

		<!-- 注意: 重启生效 -->
		<div class="px-3 py-2 rounded bg-amber-50 dark:bg-amber-500/5 border border-amber-200 dark:border-amber-500/30 text-xs text-amber-700 dark:text-amber-400">
			⚠️ 修改后需重启千寻桌面端才能生效
		</div>

		<!-- 按钮组 -->
		<div class="flex items-center justify-between pt-2 border-t border-zinc-200 dark:border-zinc-800">
			<button
				class="text-xs px-3 py-1.5 rounded text-zinc-500 hover:text-zinc-700 dark:hover:text-zinc-300"
				onclick={onReset}
			>
				重置所有
			</button>
			<div class="flex items-center gap-2">
				<button
					class="text-xs px-3 py-1.5 rounded text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800"
					onclick={onClose}
				>
					取消
				</button>
				<button
					class="text-xs px-4 py-1.5 rounded bg-amber-500 hover:bg-amber-600 text-zinc-950 font-medium"
					onclick={onSave}
				>
					保存
				</button>
			</div>
		</div>
	</div>
</Modal>
