<script lang="ts">
	// System 状态面板 (Stage 7b)
	// Dashboard 卡片: CPU / mem / uptime / conns / sessions
	// 折线图: 最近 1 分钟 conns (前端用 svg 手画)
	// 日志查看: textarea 显示最近 100 行

	import { onMount, onDestroy } from 'svelte';
	import { RefreshCw, Cpu, MemoryStick, Clock, Activity, Layers, FileText } from '@lucide/svelte';
	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import { getLogs, getMetrics, getStatus } from '$lib/api/system';
	import { authStore } from '$lib/stores/auth.svelte';
	import type { SystemMetrics, SystemStatus } from '$lib/types/api';
	import { t } from '$lib/i18n';

	let metrics = $state<SystemMetrics | null>(null);
	let status = $state<SystemStatus | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let logs = $state<string[]>([]);
	let logsLoading = $state(false);
	let logsError = $state<string | null>(null);

	let timer: ReturnType<typeof setInterval> | null = null;

	async function refreshMetrics() {
		try {
			metrics = await getMetrics();
		} catch (e) {
			error = e instanceof Error ? e.message : '加载 metrics 失败';
		}
	}

	async function refreshLogs() {
		logsLoading = true;
		logsError = null;
		try {
			const r = await getLogs(100);
			logs = r.lines ?? [];
		} catch (e) {
			logsError = e instanceof Error ? e.message : '加载日志失败';
		} finally {
			logsLoading = false;
		}
	}

	function formatUptime(s: number): string {
		if (s < 60) return `${s}s`;
		if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
		const h = Math.floor(s / 3600);
		const m = Math.floor((s % 3600) / 60);
		return `${h}h ${m}m`;
	}

	// svg 折线: 60 个点, 高度 60px, 宽度 100%
	function buildPath(history: number[] | undefined): string {
		const data = history ?? [];
		if (data.length < 2) return '';
		const max = Math.max(...data, 1);
		const w = 100;
		const h = 60;
		return data
			.map((v, i) => {
				const x = (i / (data.length - 1)) * w;
				const y = h - (v / max) * h;
				return `${i === 0 ? 'M' : 'L'}${x.toFixed(2)},${y.toFixed(2)}`;
			})
			.join(' ');
	}

	function buildArea(history: number[] | undefined): string {
		const data = history ?? [];
		if (data.length < 2) return '';
		const max = Math.max(...data, 1);
		const w = 100;
		const h = 60;
		const line = data
			.map((v, i) => {
				const x = (i / (data.length - 1)) * w;
				const y = h - (v / max) * h;
				return `${i === 0 ? 'M' : 'L'}${x.toFixed(2)},${y.toFixed(2)}`;
			})
			.join(' ');
		return `${line} L${w},${h} L0,${h} Z`;
	}

	const linePath = $derived(buildPath(metrics?.conns_history));
	const areaPath = $derived(buildArea(metrics?.conns_history));
	const historyMax = $derived(Math.max(...(metrics?.conns_history ?? [0]), 1));

	onMount(async () => {
		loading = true;
		try {
			[metrics, status] = await Promise.all([getMetrics(), getStatus()]);
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		} finally {
			loading = false;
		}
		await refreshLogs();
		timer = setInterval(refreshMetrics, 5_000);
	});

	onDestroy(() => {
		if (timer) clearInterval(timer);
	});

	// 2026-06-04 fix: 登录后自动重 fetch (见 llm/+page.svelte 注释).
	// firstRun 跳过首次 (onMount 已做 + setInterval 启动), 仅 token 变化触发.
	let firstRun = true;
	$effect(() => {
		const token = authStore.token;
		if (firstRun) {
			firstRun = false;
			return;
		}
		if (token) {
			void (async () => {
				loading = true;
				try {
					[metrics, status] = await Promise.all([getMetrics(), getStatus()]);
				} catch (e) {
					error = e instanceof Error ? e.message : '加载失败';
				} finally {
					loading = false;
				}
				await refreshLogs();
			})();
		}
	});
</script>

<PageHeader title={t('panel.system.title')} description={t('panel.system.desc')}>
	{#snippet actions()}
		<Button variant="outline" size="sm" onclick={refreshMetrics}>
			<RefreshCw class="h-3.5 w-3.5" />
			{t('common.refresh')}
		</Button>
	{/snippet}
</PageHeader>

{#if error}
	<ErrorBanner message={error} class="mb-4" />
{/if}

{#if loading}
	<Loading label={t('common.loading')} />
{:else if metrics}
	<div class="grid grid-cols-2 gap-3 md:grid-cols-5" data-testid="system-cards">
		<Card>
			<CardHeader class="flex flex-row items-center justify-between gap-2">
				<CardTitle class="text-sm">{t('panel.system.cpu')}</CardTitle>
				<Cpu class="text-muted-foreground h-4 w-4" />
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold" data-testid="metric-cpu">
					{metrics.cpu_percent?.toFixed(1) ?? '—'}%
				</p>
			</CardContent>
		</Card>

		<Card>
			<CardHeader class="flex flex-row items-center justify-between gap-2">
				<CardTitle class="text-sm">{t('panel.system.mem')}</CardTitle>
				<MemoryStick class="text-muted-foreground h-4 w-4" />
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold" data-testid="metric-mem">
					{metrics.mem_mb?.toFixed(0) ?? '—'}
				</p>
			</CardContent>
		</Card>

		<Card>
			<CardHeader class="flex flex-row items-center justify-between gap-2">
				<CardTitle class="text-sm">{t('panel.system.uptime')}</CardTitle>
				<Clock class="text-muted-foreground h-4 w-4" />
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold" data-testid="metric-uptime">
					{formatUptime(metrics.uptime_s)}
				</p>
			</CardContent>
		</Card>

		<Card>
			<CardHeader class="flex flex-row items-center justify-between gap-2">
				<CardTitle class="text-sm">{t('panel.system.active_conns')}</CardTitle>
				<Activity class="text-muted-foreground h-4 w-4" />
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold" data-testid="metric-conns">
					{metrics.active_conns}
				</p>
			</CardContent>
		</Card>

		<Card>
			<CardHeader class="flex flex-row items-center justify-between gap-2">
				<CardTitle class="text-sm">{t('panel.system.sessions')}</CardTitle>
				<Layers class="text-muted-foreground h-4 w-4" />
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold" data-testid="metric-sessions">
					{metrics.sessions.active}/{metrics.sessions.paused}/{metrics.sessions.total}
				</p>
				<p class="text-muted-foreground text-[10px]">
					{t('panel.system.sessions_active')}/{t('panel.system.sessions_paused')}/{t('panel.system.sessions_total')}
				</p>
			</CardContent>
		</Card>
	</div>

	<Card class="mt-3" data-testid="system-conns-chart">
		<CardHeader>
			<CardTitle>{t('panel.system.conns_history')}</CardTitle>
		</CardHeader>
		<CardContent>
			{#if linePath}
				<svg viewBox="0 0 100 60" preserveAspectRatio="none" class="h-20 w-full">
					<path d={areaPath} fill="var(--qianxun-accent)" fill-opacity="0.15" />
					<path d={linePath} fill="none" stroke="var(--qianxun-accent)" stroke-width="0.5" />
				</svg>
				<p class="text-muted-foreground mt-1 text-xs">
					max: {historyMax}
				</p>
			{:else}
				<p class="text-muted-foreground text-xs">—</p>
			{/if}
		</CardContent>
	</Card>

	<Card class="mt-3" data-testid="system-logs-card">
		<CardHeader class="flex flex-row items-center justify-between gap-2">
			<CardTitle>
				<FileText class="mr-1 inline h-3.5 w-3.5" />
				{t('panel.system.logs')}
			</CardTitle>
			<Button size="sm" variant="outline" onclick={refreshLogs} disabled={logsLoading}>
				<RefreshCw class="h-3 w-3" />
				{t('panel.system.logs_refresh')}
			</Button>
		</CardHeader>
		<CardContent>
			{#if logsError}
				<ErrorBanner message={logsError} />
			{:else if logsLoading}
				<Loading label={t('common.loading')} />
			{:else if logs.length === 0}
				<p class="text-muted-foreground text-xs">—</p>
			{:else}
				<textarea
					readonly
					class="bg-muted text-foreground h-64 w-full rounded-md p-2 font-mono text-[11px]"
					value={logs.join('\n')}
					data-testid="system-logs-textarea"
				></textarea>
			{/if}
		</CardContent>
	</Card>

	{#if status}
		<p class="text-muted-foreground mt-3 text-[10px]" data-testid="system-status-version">
			daemon v{status.version} · {status.stage} · {status.status}
		</p>
	{/if}
{/if}
