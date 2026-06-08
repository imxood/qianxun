<script lang="ts" module>
	// 简化的 Dialog 实现 (Stage 7a 不用 bits-ui, 自己写以减少 dep 风险).
	// 支持受控 open + ESC 关闭 + 点击 backdrop 关闭.

	import { cn } from '$lib/utils';

	export type DialogProps = {
		open: boolean;
		onOpenChange?: (open: boolean) => void;
		title?: string;
		description?: string;
		children?: import('svelte').Snippet;
		class?: string;
	};
</script>

<script lang="ts">
	let { open, onOpenChange, title, description, children, class: className = '' }: DialogProps =
		$props();

	function close() {
		onOpenChange?.(false);
	}

	function onKey(e: KeyboardEvent) {
		if (e.key === 'Escape' && open) {
			e.preventDefault();
			close();
		}
	}

	$effect(() => {
		if (typeof window === 'undefined') return;
		if (open) {
			window.addEventListener('keydown', onKey);
			return () => window.removeEventListener('keydown', onKey);
		}
	});
</script>

{#if open}
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
		role="dialog"
		aria-modal="true"
		aria-label={title}
		onclick={(e) => {
			if (e.target === e.currentTarget) close();
		}}
		onkeydown={onKey}
		tabindex={-1}
	>
		<div
			class={cn(
				'bg-card text-card-foreground border-border w-full max-w-lg rounded-lg border p-6 shadow-lg',
				className
			)}
			role="document"
		>
			{#if title}
				<h2 class="mb-1 text-lg font-semibold">{title}</h2>
			{/if}
			{#if description}
				<p class="text-muted-foreground mb-4 text-sm">{description}</p>
			{/if}
			{@render children?.()}
		</div>
	</div>
{/if}
