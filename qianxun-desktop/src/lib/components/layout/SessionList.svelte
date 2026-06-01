<script lang="ts">
	import type { Session } from "$lib/types/ipc";
	import Plus from "@lucide/svelte/icons/plus";
	import MessageSquare from "@lucide/svelte/icons/message-square";

	type Props = {
		sessions: Session[];
		activeSessionId?: string;
		onSelectSession?: (id: string) => void;
	};

	let { sessions, activeSessionId, onSelectSession }: Props = $props();

	function formatTime(iso: string): string {
		const t = new Date(iso).getTime();
		const diff = Date.now() - t;
		const m = Math.floor(diff / 60_000);
		if (m < 1) return "刚刚";
		if (m < 60) return `${m}m ago`;
		const h = Math.floor(m / 60);
		if (h < 24) return `${h}h ago`;
		const d = Math.floor(h / 24);
		return `${d}d ago`;
	}
</script>

<section class="flex flex-col gap-2">
	<header class="flex items-center justify-between">
		<h2 class="text-sm font-semibold">会话</h2>
		<button
			class="text-muted-foreground hover:text-foreground"
			title="新建会话 (Stage 2)"
			aria-label="新建会话"
		>
			<Plus class="size-3.5" />
		</button>
	</header>

	{#each sessions as session (session.id)}
		<button
			type="button"
			class="block w-full cursor-pointer rounded p-2 text-left hover:bg-accent hover:text-accent-foreground"
			class:bg-accent={session.id === activeSessionId}
			onclick={() => onSelectSession?.(session.id)}
		>
			<div class="flex items-start gap-1.5">
				<MessageSquare class="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
				<div class="min-w-0 flex-1">
					<div class="truncate text-sm font-medium">{session.title}</div>
					<div class="mt-0.5 flex items-center gap-1.5 text-xs text-muted-foreground">
						<span>{session.model}</span>
						<span>·</span>
						<span>{session.message_count} 条</span>
						<span>·</span>
						<span>{formatTime(session.last_active_at)}</span>
					</div>
				</div>
			</div>
		</button>
	{/each}

	{#if sessions.length === 0}
		<div class="px-2 py-1 text-xs text-muted-foreground">暂无会话</div>
	{/if}
</section>
