<script lang="ts">
	// qianxun-desktop/src/lib/components/col2/ChatStream.svelte
	// 通用 chat 流渲染 + 输入框. 复用给 session 和 sub_session 两个分支.
	// - 渲染 messages (user/assistant + followup 角标)
	// - assistant 消息的 plan_ref 走 PlanBlock 渲染
	// - 接受 onSend 回调, 由调用方决定发到 session 还是 sub_session
	// - placeholder 跟 mode 区分 (Active 用 "输入消息...", followup 用 "追问 (不执行)...")

	import Icon from '../shared/Icon.svelte';
	import Avatar from '../shared/Avatar.svelte';
	import PlanBlock from './PlanBlock.svelte';
	import { planStore } from '$lib/stores/plan.svelte';
	import type { Message } from '$lib/types/entity';

	let {
		messages,
		onSend,
		placeholder = '输入消息... (Enter 发送 · Shift+Enter 换行)',
		mode = 'task' as 'task' | 'followup',
	}: {
		messages: Message[];
		onSend: (text: string) => void | Promise<void>;
		placeholder?: string;
		mode?: 'task' | 'followup';
	} = $props();

	let inputEl: HTMLTextAreaElement | undefined = $state();
	let inputValue = $state('');

	async function handleSend() {
		if (!inputValue.trim()) return;
		const text = inputValue;
		inputValue = '';
		await onSend(text);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && !e.shiftKey) {
			e.preventDefault();
			handleSend();
		}
	}

	function followupHint() {
		// 追问模式下: 简化 placeholder
		return '追问 (不执行任务, 仅聊天) · Enter 发送';
	}

	const finalPlaceholder = $derived(mode === 'followup' ? followupHint() : placeholder);
</script>

<div class="flex-1 overflow-y-auto px-4 py-6 space-y-4 bg-zinc-50 dark:bg-zinc-950">
	{#each messages as msg (msg.id)}
		{#if msg.role === 'user'}
			<div class="flex gap-3 max-w-3xl">
				<Avatar kind="user" />
				<div class="flex-1 pt-1">
					<div class="flex items-center gap-2 mb-1">
						<span class="text-xs text-zinc-500">maxu</span>
						{#if msg.kind === 'followup'}
							<span class="text-[10px] px-1.5 py-0.5 rounded bg-zinc-200 dark:bg-zinc-800 text-zinc-500 dark:text-zinc-400">追问</span>
						{/if}
					</div>
					<p class="text-sm text-zinc-900 dark:text-zinc-100 leading-relaxed whitespace-pre-wrap">{msg.content}</p>
				</div>
			</div>
		{:else if msg.role === 'assistant'}
			<div class="flex gap-3 max-w-3xl">
				<Avatar kind="assistant" />
				<div class="flex-1 pt-1">
					<div class="flex items-center gap-2 mb-1">
						<span class="text-xs text-zinc-500">小寻 (主 Agent)</span>
						{#if msg.kind === 'followup'}
							<span class="text-[10px] px-1.5 py-0.5 rounded bg-zinc-200 dark:bg-zinc-800 text-zinc-500 dark:text-zinc-400">追问</span>
						{/if}
					</div>
					{#if msg.plan_ref}
						{@const plan = planStore.get(msg.plan_ref)}
						{#if plan}
							<div class="ml-0 max-w-3xl">
								<PlanBlock {plan} />
							</div>
						{/if}
					{:else}
						<p class="text-sm text-zinc-800 dark:text-zinc-200 leading-relaxed whitespace-pre-wrap">
							{msg.content}{#if msg.streaming}<span class="inline-block w-2 h-4 bg-amber-500 ml-0.5 align-middle animate-pulse"></span>{/if}
						</p>
					{/if}
				</div>
			</div>
		{/if}
	{/each}
</div>

<div class="p-3 border-t border-zinc-200 dark:border-zinc-800 flex-shrink-0 bg-white dark:bg-zinc-950">
	<div class="rounded-lg border border-amber-500/50 bg-white dark:bg-zinc-900 focus-within:border-amber-500 transition shadow-sm">
		<textarea
			bind:this={inputEl}
			bind:value={inputValue}
			onkeydown={handleKeydown}
			rows="2"
			placeholder={finalPlaceholder}
			class="w-full bg-transparent text-sm text-zinc-900 dark:text-zinc-100 placeholder-zinc-400 dark:placeholder-zinc-500 px-3 pt-2.5 pb-1 resize-none focus:outline-none"
		></textarea>
		<div class="px-2 pb-2 flex items-center gap-1">
			<button class="p-1.5 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-500" aria-label="附件">
				<Icon name="at-sign" class="w-3.5 h-3.5" />
			</button>
			<button class="p-1.5 rounded hover:bg-zinc-100 dark:hover:bg-zinc-800 text-zinc-500" aria-label="始终授权">
				<Icon name="shield-check" class="w-3.5 h-3.5" />
			</button>
			<div class="flex-1"></div>
			<button
				class="p-1.5 rounded bg-amber-500 hover:bg-amber-600 text-zinc-950 disabled:opacity-50"
				disabled={!inputValue.trim()}
				onclick={handleSend}
				aria-label="发送"
			>
				<Icon name="arrow-up" class="w-3.5 h-3.5" />
			</button>
		</div>
	</div>
</div>
