<script lang="ts">
	import Modal from '../shared/Modal.svelte';
	import Icon from '../shared/Icon.svelte';

	let { open, onClose, items = [] }: { open: boolean; onClose: () => void; items?: { content: string }[] } = $props();
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
			<button class="text-xs px-3 py-1.5 rounded text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800" onclick={onClose}>跳过</button>
			<button class="text-xs px-3 py-1.5 rounded text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800" onclick={onClose}>修改</button>
			<button class="text-xs px-4 py-1.5 rounded bg-amber-500 hover:bg-amber-600 text-zinc-950 font-medium" onclick={onClose}>
				沉淀 {items.length} 条
			</button>
		</div>
	</div>
</Modal>
