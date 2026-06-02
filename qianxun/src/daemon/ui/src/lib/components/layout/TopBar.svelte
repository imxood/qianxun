<script lang="ts">
	// Stage 7a TopBar — daemon 状态指示 + token 配置
	// token 配置: 显示当前已配置, 点击编辑

	import { Circle, KeyRound, LogOut, RefreshCw } from '@lucide/svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { onMount } from 'svelte';

	type Props = {
		onConfigureToken?: () => void;
	};

	let { onConfigureToken }: Props = $props();

	type DaemonState = 'connected' | 'unknown' | 'offline';
	let daemonState: DaemonState = $state<DaemonState>('unknown');
	let version = $state<string>('');

	async function refresh() {
		try {
			const r = await fetch('/v1/system/health', {
				headers: authStore.token
					? { Authorization: `Bearer ${authStore.token}` }
					: {}
			});
			if (r.ok) {
				daemonState = 'connected';
				const s = await fetch('/v1/system/status', {
					headers: authStore.token
						? { Authorization: `Bearer ${authStore.token}` }
						: {}
				});
				if (s.ok) {
					const j = (await s.json()) as { version?: string };
					version = j.version ?? '';
				}
			} else {
				daemonState = r.status === 401 ? 'unknown' : 'offline';
			}
		} catch {
			daemonState = 'offline';
		}
	}

	function logout() {
		authStore.clear();
	}

	onMount(() => {
		void refresh();
		const t = setInterval(refresh, 10_000);
		return () => clearInterval(t);
	});

	const stateLabel = $derived(
		daemonState === 'connected'
			? 'Daemon 已连接'
			: daemonState === 'offline'
				? 'Daemon 离线'
				: '未鉴权'
	);
	const stateColor = $derived(
		daemonState === 'connected' ? '#22c55e' : daemonState === 'offline' ? '#ef4444' : '#a3a3a3'
	);
	const tokenMask = $derived(
		authStore.token
			? authStore.token.slice(0, 6) + '…' + authStore.token.slice(-4)
			: '未配置'
	);
</script>

<header
	class="bg-background flex h-14 items-center justify-between border-b px-4"
	aria-label="顶部状态栏"
>
	<div class="flex items-center gap-2 text-sm">
		<Circle class="h-3 w-3" style="color: {stateColor};" fill={stateColor} />
		<span class="font-medium">{stateLabel}</span>
		{#if version}
			<span class="text-muted-foreground text-xs">v{version}</span>
		{/if}
		<button
			type="button"
			class="text-muted-foreground hover:text-foreground ml-1"
			title="刷新状态"
			aria-label="刷新状态"
			onclick={refresh}
		>
			<RefreshCw class="h-3.5 w-3.5" />
		</button>
	</div>

	<div class="flex items-center gap-2 text-sm">
		<div class="text-muted-foreground flex items-center gap-1.5">
			<KeyRound class="h-3.5 w-3.5" />
			<code class="bg-muted rounded px-1.5 py-0.5 text-xs">{tokenMask}</code>
		</div>
		<button
			type="button"
			class="border-input hover:bg-accent rounded-md border px-2.5 py-1 text-xs"
			onclick={onConfigureToken}
			data-testid="topbar-configure-token"
		>
			{authStore.token ? '更换' : '设置'}
		</button>
		{#if authStore.token}
			<button
				type="button"
				class="text-muted-foreground hover:text-foreground"
				title="登出"
				aria-label="登出"
				onclick={logout}
			>
				<LogOut class="h-3.5 w-3.5" />
			</button>
		{/if}
	</div>
</header>
