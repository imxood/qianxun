<script lang="ts">
	// 2026-06-09 加: 启动 splash 屏. 解决 "tauri 启动后 10+ 秒空白" 问题.
	// 之前: +page.svelte 模板直接渲染 ThreeColumnLayout, 期间 uiStore.activeView 默认
	// 'session' 引用不存在的 'sess_jwt_auth', 显示"还没有会话"静态 div. 用户看不到
	// 任何"正在加载"反馈, 不知道是启动慢还是卡死.
	//
	// 2026-06-09 体验打磨:
	//   - 4 step 都有 icon + 文字细节, 用户知道后台在做什么
	//   - 进度条平滑 (0→100 transition, 不闪)
	//   - 淡入动画 (300ms)
	//   - 错误展示用顶部 banner, 不阻塞布局
	//   - 失败重试按钮

	import Icon from '../shared/Icon.svelte';

	let {
		step = 'starting' as 'starting' | 'connecting' | 'loading' | 'ready' | 'failed',
		progress = 0 as number, // 0-100
		error = null as string | null,
		onRetry = null as (() => void) | null,
	} = $props();

	// 每步的 icon + 详细说明 (体验打磨: 用户知道后台在做什么)
	const STEPS = [
		{ key: 'starting', icon: 'zap', label: '启动千寻运行时', detail: '加载配置和 provider' },
		{ key: 'connecting', icon: 'loader', label: '连接 desktop bridge', detail: '注册 session_event 监听' },
		{ key: 'loading', icon: 'layers', label: '加载会话列表', detail: '从 SQLite 恢复持久化数据' },
		{ key: 'ready', icon: 'check', label: '就绪', detail: '可以开始对话' },
	] as const;

	const currentStepIndex = $derived.by(() => {
		const i = STEPS.findIndex((s) => s.key === step);
		return i < 0 ? 0 : i;
	});

	const currentStep = $derived(STEPS[currentStepIndex]);
</script>

<!-- 2026-06-09 体验打磨: 启动错误用顶部 banner, 用户始终能看到主布局占位 (避免"卡死" 感) -->
{#if error}
	<div
		class="fixed top-0 inset-x-0 z-[60] bg-red-50 dark:bg-red-950/90 border-b border-red-200 dark:border-red-800 px-4 py-2 flex items-center gap-3 text-red-800 dark:text-red-200"
		role="alert"
	>
		<Icon name="alert-circle" class="w-4 h-4 flex-shrink-0" />
		<p class="text-xs flex-1">{error}</p>
		{#if onRetry}
			<button
				class="text-xs px-2 py-0.5 rounded border border-red-300 dark:border-red-700 hover:bg-red-100 dark:hover:bg-red-900 transition-colors"
				onclick={onRetry}
			>
				重试
			</button>
		{/if}
	</div>
{/if}

<div
	class="fixed inset-0 z-50 flex flex-col items-center justify-center bg-zinc-50 dark:bg-zinc-950 animate-[fadein_300ms_ease-out]"
	role="status"
	aria-live="polite"
>
	<!-- Logo / 标题 -->
	<div class="flex flex-col items-center gap-3 mb-10">
		<div class="relative">
			<Icon name="zap" class="w-12 h-12 text-amber-500" />
			<!-- 2026-06-09 体验打磨: logo 后面一圈脉冲光晕, 表示"正在工作" -->
			<div class="absolute inset-0 -m-2 rounded-full bg-amber-500/20 animate-ping"></div>
		</div>
		<h1 class="text-2xl font-bold text-zinc-800 dark:text-zinc-100 tracking-wide">千寻</h1>
		<p class="text-sm text-zinc-500 dark:text-zinc-400">Qianxun · 个人 AI 系统</p>
	</div>

	<!-- 进度条 -->
	<div class="w-72">
		<div class="h-1.5 bg-zinc-200 dark:bg-zinc-800 rounded-full overflow-hidden">
			<div
				class="h-full bg-gradient-to-r from-amber-400 to-amber-500 transition-all duration-500 ease-out"
				style="width: {progress}%"
			></div>
		</div>
		<p class="mt-2 text-center text-xs text-zinc-500 dark:text-zinc-400 tabular-nums">
			{currentStep?.label ?? '就绪'} · {progress}%
		</p>
	</div>

	<!-- 2026-06-09 体验打磨: 4 step 列表 (icon + 文字 + 详情), 完成的步骤变绿, 当前的变蓝, 未到的灰 -->
	<div class="mt-8 w-72 space-y-2">
		{#each STEPS as s, i (s.key)}
			{@const is_done = i < currentStepIndex}
			{@const is_current = i === currentStepIndex}
			<div class="flex items-start gap-2.5 transition-opacity {is_current || is_done ? 'opacity-100' : 'opacity-40'}">
				<div class="flex-shrink-0 mt-0.5 w-5 h-5 rounded-full flex items-center justify-center transition-colors
					{is_done ? 'bg-green-500 text-white' : is_current ? 'bg-amber-500 text-white' : 'bg-zinc-200 dark:bg-zinc-800'}"
				>
					{#if is_done}
						<Icon name="check" class="w-3 h-3" />
					{:else if is_current}
						<!-- 2026-06-09 体验打磨: 当前步骤 spinner 旋转, 表示"正在执行" -->
						<div class="w-2 h-2 border-2 border-white border-t-transparent rounded-full animate-spin"></div>
					{/if}
				</div>
				<div class="flex-1 min-w-0">
					<p class="text-xs font-medium {is_done ? 'text-green-700 dark:text-green-300' : is_current ? 'text-zinc-800 dark:text-zinc-100' : 'text-zinc-500 dark:text-zinc-400'}">
						{s.label}
					</p>
					{#if is_current || is_done}
						<p class="text-[10px] text-zinc-500 dark:text-zinc-500 mt-0.5">{s.detail}</p>
					{/if}
				</div>
			</div>
		{/each}
	</div>
</div>

<!-- 2026-06-09 体验打磨: fade-in keyframes (避免在 <style> 块定义时污染全局) -->
<style>
	@keyframes fadein {
		from { opacity: 0; }
		to { opacity: 1; }
	}
</style>
