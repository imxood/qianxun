<script lang="ts">
	// Stage 9c — SessionList (Web Console 复制 Tauri)
	// 跟 qianxun-desktop/src/lib/components/layout/SessionList.svelte 一致
	// 区别: 接 webui 的 ChatSession (lib/types/chat.ts), 路径用 $lib/types/chat

	import type { ChatSession } from '$lib/types/chat';
	import Plus from '@lucide/svelte/icons/plus';
	import MessageSquare from '@lucide/svelte/icons/message-square';
	import { t } from '$lib/i18n';

	type Props = {
		sessions: ChatSession[];
		activeSessionId?: string | null;
		onSelectSession?: (id: string) => void;
		onCreateSession?: () => void;
	};

	let { sessions, activeSessionId, onSelectSession, onCreateSession }: Props = $props();

	function formatTime(iso: string): string {
		const t = new Date(iso).getTime();
		const diff = Date.now() - t;
		const m = Math.floor(diff / 60_000);
		if (m < 1) return '刚刚';
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
			type="button"
			class="text-muted-foreground hover:text-foreground"
			onclick={() => onCreateSession?.()}
			title="新建会话"
			aria-label="新建会话"
			data-testid="session-list-new"
		>
			<Plus class="size-3.5" />
		</button>
	</header>

	{#each sessions as session (session.id)}
		<button
			type="button"
			class="hover:bg-accent hover:text-accent-foreground block w-full cursor-pointer rounded p-2 text-left"
			class:bg-accent={session.id === activeSessionId}
			onclick={() => onSelectSession?.(session.id)}
			data-testid={`session-item-${session.id}`}
		>
			<div class="flex items-start gap-1.5">
				<MessageSquare class="text-muted-foreground mt-0.5 size-3.5 shrink-0" />
				<div class="min-w-0 flex-1">
					<div class="truncate text-sm font-medium">
						{session.id.slice(0, 12)}…
					</div>
					<div class="text-muted-foreground mt-0.5 flex items-center gap-1.5 text-xs">
						<span class="font-mono">{session.model}</span>
						<span>·</span>
						<span>{session.message_count} 条</span>
						<span>·</span>
						<span>{formatTime(session.last_active)}</span>
					</div>
				</div>
			</div>
		</button>
	{/each}

	{#if sessions.length === 0}
		<div class="text-muted-foreground px-2 py-1 text-xs">暂无会话</div>
	{/if}
</section>
