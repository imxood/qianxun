<script lang="ts">
	// 2026-06-12 (Phase B.4): 按钮接 onApprove/onReject 回调, 父组件注入实际审批逻辑.
	// 当前无调用方 (后续 P1.5 / v0.4 接入 tool approval 流时启用), 接口已对齐.
	import Modal from '../shared/Modal.svelte';
	import Icon from '../shared/Icon.svelte';

	let {
		open,
		onClose,
		request,
		onApprove,
		onReject,
	}: {
		open: boolean;
		onClose: () => void;
		request: { kind: 'file' | 'command' | 'network'; detail: string };
		onApprove: (remember: boolean) => void;
		onReject: (remember: boolean) => void;
	} = $props();

	let remember = $state(false);
</script>

<Modal {open} {onClose} title="需要你的批准" maxWidth="max-w-lg">
	<div class="space-y-3">
		<div class="px-3 py-3 rounded-md border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/5">
			<div class="flex items-center gap-2 mb-1.5">
				<Icon
					name={request.kind === 'file' ? 'file-code' : request.kind === 'command' ? 'cpu' : 'info'}
					class="w-4 h-4 text-amber-500"
				/>
				<span class="text-sm font-medium text-zinc-800 dark:text-zinc-200">
					{request.kind === 'file' ? '写入文件' : request.kind === 'command' ? '执行命令' : '网络请求'}
				</span>
			</div>
			<pre class="text-xs font-mono text-zinc-600 dark:text-zinc-400 whitespace-pre-wrap break-all bg-white dark:bg-zinc-950 px-2 py-1.5 rounded">{request.detail}</pre>
		</div>

		<label class="flex items-center gap-2 text-xs text-zinc-600 dark:text-zinc-400">
			<input type="checkbox" class="accent-amber-500" bind:checked={remember} />
			记住这次, 后续同类操作不再询问
		</label>

		<div class="flex items-center justify-end gap-2 pt-2 border-t border-zinc-200 dark:border-zinc-800">
			<button
				class="text-xs px-3 py-1.5 rounded text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800"
				onclick={() => onReject(remember)}
			>
				拒绝
			</button>
			<button
				class="text-xs px-4 py-1.5 rounded bg-amber-500 hover:bg-amber-600 text-zinc-950 font-medium"
				onclick={() => onApprove(remember)}
			>
				批准
			</button>
		</div>
	</div>
</Modal>
