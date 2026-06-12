<script lang="ts">
	// 2026-06-12 (Phase C): 三个按钮接 dismiss 行为 + localStorage "已读" 标记.
	// 关键设计: 三个按钮都只 dismiss, 不调后端 (experience 沉淀 API 暂无, 留 v0.4).
	// localStorage key: qianxun.experience.dismissedAt, 存 ISO 时间戳.
	//
	// 2026-06-12 (批次 3.5): onClose 改可选 + 默认 noop, 跟 ApprovalModal (3.3) 同模式.
	// 规范 10 命名准确: 0 caller 状态也允许 mount. 本 modal 无内部 $state, 不需要
	// 跨 open 复位 (items 全受控, 由父组件 prop 决定).
	import Modal from '../shared/Modal.svelte';
	import Icon from '../shared/Icon.svelte';

	let {
		open = false,
		onClose = () => {},
		items = [],
	}: {
		open?: boolean;
		onClose?: () => void;
		items?: { content: string }[];
	} = $props();

	function dismiss(reason: 'skip' | 'modify' | 'commit') {
		try {
			localStorage.setItem(
				'qianxun.experience.dismissedAt',
				JSON.stringify({ reason, ts: new Date().toISOString() }),
			);
		} catch {
			// localStorage 不可用时静默 (P2 报告走 Phase A.3 统一通道)
		}
		onClose();
	}
</script>

<Modal {open} {onClose} title="建议沉淀项目经验" maxWidth="max-w-lg">
	<div class="space-y-3">
		<div class="px-3 py-2 rounded-md bg-amber-50 dark:bg-amber-500/5 border border-amber-200 dark:border-amber-500/30 flex items-start gap-2">
			<Icon name="info" class="w-4 h-4 text-amber-500 mt-0.5 flex-shrink-0" />
			<p class="text-xs text-zinc-700 dark:text-zinc-300">从这次 Plan 完成中学到 3 条, 可写入项目经验供下次复用.</p>
		</div>

		<div class="space-y-2">
			{#each items as item, i (i)}
				<label class="flex items-start gap-2 px-2 py-1.5 rounded hover:bg-zinc-50 dark:hover:bg-zinc-800/30 cursor-pointer">
					<input type="checkbox" checked class="mt-1 accent-amber-500" />
					<span class="text-sm text-zinc-700 dark:text-zinc-300">{item.content}</span>
				</label>
			{/each}
		</div>

		<div class="flex items-center justify-end gap-2 pt-2 border-t border-zinc-200 dark:border-zinc-800">
			<button class="text-xs px-3 py-1.5 rounded text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800" onclick={() => dismiss('skip')}>跳过</button>
			<button class="text-xs px-3 py-1.5 rounded text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800" onclick={() => dismiss('modify')}>修改</button>
			<button class="text-xs px-4 py-1.5 rounded bg-amber-500 hover:bg-amber-600 text-zinc-950 font-medium" onclick={() => dismiss('commit')}>
				沉淀 {items.length} 条
			</button>
		</div>
	</div>
</Modal>
