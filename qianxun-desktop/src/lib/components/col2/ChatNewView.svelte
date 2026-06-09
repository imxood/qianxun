<script lang="ts">
	// 2026-06-09 加: 千寻的 "新对话" 入口页.
	//
	// 设计原则 (UI/UX):
	// 1. **极简居中** — 单卡片, 视窗垂直水平居中, 不放 Logo / 欢迎语 (顶部 TopBar 已有)
	// 2. **千寻风格** — 圆角 2xl, 浅边框 + 聚焦时琥珀色描边, 弱阴影
	// 3. **亮暗双适配** — 颜色一律用 zinc-* 系列 (亮 zinc-50/200/900, 暗 zinc-900/800/950)
	// 4. **工具栏极简化** — 只 1 个项目下拉 + 1 个发送按钮. 模式/分支/思考/模型等
	//    复杂控件等 session 创建后再展开, 避免新对话时决策负担
	// 5. **键盘友好** — Enter 发送, Shift+Enter 换行, Esc 关下拉, 自动 focus textarea
	//
	// 数据流:
	// - project 下拉: projectStore.all (从 listSessions('all') derive, 按 session 数排序)
	// - "选择文件夹..." 项: 调 Tauri dialog plugin, 选完后路径作为 project_id
	// - 发送: chatStore.send(null, text) → lazy create session + sendMessage (透传 project_root)

	import Icon from '../shared/Icon.svelte';
	import { uiStore } from '$lib/stores/ui.svelte';
	import { chatStore } from '$lib/stores/chat.svelte';
	import { projectStore } from '$lib/stores/project.svelte';

	// 2026-06-09 加: 从 Tauri dialog plugin 调原生文件夹选择器.
	// Tauri 2.x plugin JS 端用 @tauri-apps/plugin-dialog (前端单独 dep).
	// 这里用动态 import 避免 web dev 模式 (非 Tauri) 报错.
	async function pickFolder(): Promise<string | null> {
		try {
			const { open } = await import('@tauri-apps/plugin-dialog');
			const selected = await open({ directory: true, multiple: false, title: '选择项目文件夹' });
			return typeof selected === 'string' ? selected : null;
		} catch (e) {
			// web dev 模式 (非 Tauri) → plugin 不可用, 弹 toast 提示
			console.warn('[ChatNewView] pickFolder failed (web dev mode?):', e);
			return null;
		}
	}

	const view = $derived(uiStore.activeView);
	const currentProjectId = $derived(view.kind === 'new' ? view.project_id : null);
	const currentProject = $derived(
		currentProjectId ? projectStore.get(currentProjectId) : null
	);

	let inputEl: HTMLTextAreaElement | undefined = $state();
	let inputValue = $state('');
	let projectDropdownOpen = $state(false);
	let dropdownContainerEl: HTMLDivElement | undefined = $state();

	$effect(() => {
		if (view.kind === 'new' && inputEl) {
			setTimeout(() => inputEl?.focus(), 50);
		}
	});

	async function handleSend() {
		const text = inputValue.trim();
		if (!text) return;
		inputValue = '';
		projectDropdownOpen = false;
		// chatStore.send(null, text) 触发 lazy create session
		await chatStore.send(null, text);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			handleSend();
		}
		if (e.key === 'Escape') {
			projectDropdownOpen = false;
		}
	}

	function selectProject(id: string | null) {
		uiStore.switchToNew(id);
		projectDropdownOpen = false;
		setTimeout(() => inputEl?.focus(), 50);
	}

	// 2026-06-09 加: 调 Tauri dialog 选文件夹, 选完作为新 project_id.
	async function handlePickFolder() {
		projectDropdownOpen = false;
		const path = await pickFolder();
		if (path) {
			uiStore.switchToNew(path);
			setTimeout(() => inputEl?.focus(), 50);
		}
	}

	// 点击外部关下拉
	function handleWindowMouseDown(e: MouseEvent) {
		if (!projectDropdownOpen) return;
		const target = e.target as Node;
		if (dropdownContainerEl && !dropdownContainerEl.contains(target)) {
			projectDropdownOpen = false;
		}
	}
</script>

<svelte:window onmousedown={handleWindowMouseDown} />

<!-- 居中卡片 -->
<main class="flex-1 flex items-center justify-center px-4 bg-zinc-50 dark:bg-zinc-950">
	<div class="w-full max-w-2xl">
		<!-- 卡片: 亮 = 白底+zinc-200 边框, 暗 = zinc-900+zinc-800 边框 -->
		<div class="rounded-2xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 shadow-sm transition-all duration-150 focus-within:border-amber-500/50 focus-within:shadow-md focus-within:shadow-amber-500/5">
			<textarea
				bind:this={inputEl}
				bind:value={inputValue}
				onkeydown={handleKeydown}
				placeholder="输入消息... (Enter 发送 · Shift+Enter 换行)"
				rows="3"
				class="w-full bg-transparent px-5 pt-4 pb-2 text-sm leading-relaxed text-zinc-900 dark:text-zinc-100 placeholder-zinc-400 dark:placeholder-zinc-600 resize-none focus:outline-none"
			></textarea>

			<!-- 底部工具行 -->
			<div class="flex items-center justify-between px-2 pb-2">
				<!-- 左: 项目下拉 (相对定位容器, 含面板) -->
				<div class="relative" bind:this={dropdownContainerEl}>
					<button
						type="button"
						onclick={() => (projectDropdownOpen = !projectDropdownOpen)}
						class="flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors"
						aria-haspopup="listbox"
						aria-expanded={projectDropdownOpen}
					>
						<Icon name="folder" class="w-3.5 h-3.5 {currentProject ? 'text-amber-500' : 'text-zinc-400'}" />
						<span>{currentProject?.name ?? '选择项目'}</span>
						<Icon name="chevron-down" class="w-3 h-3 opacity-50" />
					</button>

					{#if projectDropdownOpen}
						<div
							class="absolute bottom-full left-0 mb-2 w-72 max-h-96 overflow-y-auto rounded-xl border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 shadow-xl py-1 z-20"
							role="listbox"
						>
							<!-- "无项目" 选项 (灰色, 区分项目) -->
							<button
								type="button"
								class="w-full flex items-center gap-2 px-3 py-2 text-xs text-zinc-500 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors text-left"
								onclick={() => selectProject(null)}
							>
								<Icon name="folder" class="w-3.5 h-3.5 opacity-50" />
								<span class="flex-1">无项目</span>
								{#if currentProjectId === null}
									<Icon name="check" class="w-3.5 h-3.5 text-amber-500" />
								{/if}
							</button>

							<!-- 2026-06-09 加: "选择文件夹..." 项, 调 Tauri dialog 打开原生文件夹选择器 -->
							<button
								type="button"
								class="w-full flex items-center gap-2 px-3 py-2 text-xs text-amber-600 dark:text-amber-400 hover:bg-amber-50 dark:hover:bg-amber-500/10 transition-colors text-left font-medium"
								onclick={handlePickFolder}
							>
								<Icon name="folder-plus" class="w-3.5 h-3.5" />
								<span class="flex-1">选择文件夹...</span>
							</button>

							{#if projectStore.all.length > 0}
								<div class="border-t border-zinc-100 dark:border-zinc-800 my-1"></div>
								<div class="px-3 py-1.5 text-[10px] text-zinc-400 dark:text-zinc-500 uppercase tracking-wider font-medium">
									最近项目
								</div>
								{#each projectStore.all as project (project.id)}
									<button
										type="button"
										class="w-full flex items-center gap-2 px-3 py-2 text-xs text-zinc-700 dark:text-zinc-200 hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors text-left"
										onclick={() => selectProject(project.id)}
									>
										<Icon name="folder" class="w-3.5 h-3.5 text-amber-500" />
										<span class="flex-1 truncate">{project.name}</span>
										<span class="text-[10px] text-zinc-400 dark:text-zinc-500 tabular-nums">
											{project.session_count}
										</span>
										{#if currentProjectId === project.id}
											<Icon name="check" class="w-3.5 h-3.5 text-amber-500" />
										{/if}
									</button>
								{/each}
							{/if}
						</div>
					{/if}
				</div>

				<!-- 右: 发送按钮 -->
				<button
					type="button"
					onclick={handleSend}
					disabled={!inputValue.trim()}
					class="w-8 h-8 rounded-md flex items-center justify-center transition-colors {inputValue.trim()
						? 'bg-amber-500 hover:bg-amber-600 text-zinc-950'
						: 'bg-zinc-200 dark:bg-zinc-800 text-zinc-400 dark:text-zinc-600 cursor-not-allowed'}"
					aria-label="发送"
				>
					<Icon name="arrow-up" class="w-4 h-4" />
				</button>
			</div>
		</div>
	</div>
</main>
