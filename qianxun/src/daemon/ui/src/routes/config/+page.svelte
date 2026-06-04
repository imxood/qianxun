<script lang="ts">
	// Config 管理面板 (Stage 7b)
	// 当前 read-only 视图 (Stage 7c 再做 edit + 提交)

	import { onMount } from 'svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import { RefreshCw, AlertTriangle, CheckCircle2 } from '@lucide/svelte';
	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardDescription from '$lib/components/ui/card/CardDescription.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Badge from '$lib/components/ui/badge/Badge.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import { getConfig } from '$lib/api/config';
	import type { ResolvedConfigView } from '$lib/types/api';
	import { t } from '$lib/i18n';

	let config = $state<ResolvedConfigView | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

	async function refresh() {
		loading = true;
		error = null;
		try {
			config = await getConfig();
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void refresh();
	});

	// 2026-06-04 fix: 登录后自动重 fetch (见 llm/+page.svelte 注释)
	let firstRun = true;
	$effect(() => {
		const token = authStore.token;
		if (firstRun) {
			firstRun = false;
			return;
		}
		if (token) {
			void refresh();
		}
	});
</script>

<PageHeader title={t('panel.config.title')} description={t('panel.config.desc')}>
	{#snippet actions()}
		<Button variant="outline" size="sm" onclick={refresh}>
			<RefreshCw class="h-3.5 w-3.5" />
			{t('common.refresh')}
		</Button>
	{/snippet}
</PageHeader>

<div
	class="border-border bg-muted/30 text-muted-foreground mb-3 flex items-center gap-2 rounded-md border p-2 text-xs"
	data-testid="config-readonly-banner"
>
	<AlertTriangle class="h-3.5 w-3.5" />
	{t('panel.config.readonly')}
</div>

{#if error}
	<ErrorBanner message={error} class="mb-4" />
{/if}

{#if loading}
	<Loading label={t('common.loading')} />
{:else if config}
	<div class="grid grid-cols-1 gap-3 md:grid-cols-2" data-testid="config-grid">
		<Card>
			<CardHeader>
				<CardTitle>{t('panel.config.active_provider')}</CardTitle>
				<CardDescription>当前生效的 LLM provider</CardDescription>
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold" data-testid="config-active-provider">
					{config.active_provider}
				</p>
			</CardContent>
		</Card>

		<Card>
			<CardHeader>
				<CardTitle>{t('panel.config.log_level')}</CardTitle>
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold">{config.log_level}</p>
			</CardContent>
		</Card>

		<Card>
			<CardHeader>
				<CardTitle>{t('panel.config.max_sessions')}</CardTitle>
			</CardHeader>
			<CardContent>
				<p class="font-mono text-2xl font-semibold">{config.max_sessions}</p>
			</CardContent>
		</Card>

		<Card>
			<CardHeader>
				<CardTitle>{t('panel.config.memory_dir')}</CardTitle>
			</CardHeader>
			<CardContent>
				<p class="font-mono text-sm break-all">{config.memory_dir ?? '—'}</p>
			</CardContent>
		</Card>

		<Card class="md:col-span-2">
			<CardHeader>
				<CardTitle>{t('panel.config.providers')}</CardTitle>
			</CardHeader>
			<CardContent class="p-0">
				{#if config.providers.length === 0}
					<p class="text-muted-foreground p-3 text-xs">—</p>
				{:else}
					<div class="overflow-x-auto">
						<table class="w-full text-sm" data-testid="config-providers-table">
							<thead class="bg-muted text-muted-foreground border-b text-left">
								<tr>
									<th class="px-3 py-2 font-medium">ID</th>
									<th class="px-3 py-2 font-medium">Provider</th>
									<th class="px-3 py-2 font-medium">Model</th>
									<th class="px-3 py-2 font-medium">Base URL</th>
									<th class="px-3 py-2 font-medium">Key</th>
									<th class="px-3 py-2 font-medium">Active</th>
								</tr>
							</thead>
							<tbody>
								{#each config.providers as p (p.id)}
									<tr class="border-b">
										<td class="px-3 py-2 font-mono text-xs">{p.id}</td>
										<td class="px-3 py-2 font-mono text-xs">{p.provider}</td>
										<td class="px-3 py-2 font-mono text-xs">{p.model}</td>
										<td class="px-3 py-2 font-mono text-xs break-all">{p.base_url ?? '—'}</td>
										<td class="px-3 py-2">
											{#if p.has_key}
												<CheckCircle2 class="inline h-3.5 w-3.5 text-green-600 dark:text-green-400" />
											{:else}
												<span class="text-muted-foreground text-xs">—</span>
											{/if}
										</td>
										<td class="px-3 py-2">
											{#if p.active}
												<Badge variant="success">active</Badge>
											{:else}
												<Badge variant="outline">inactive</Badge>
											{/if}
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</CardContent>
		</Card>

		<Card class="md:col-span-2">
			<CardHeader>
				<CardTitle>{t('panel.config.skills_dirs')}</CardTitle>
			</CardHeader>
			<CardContent>
				{#if config.skills_dirs.length === 0}
					<p class="text-muted-foreground text-xs">—</p>
				{:else}
					<ul class="flex flex-col gap-1">
						{#each config.skills_dirs as d, i (i)}
							<li class="bg-muted rounded px-2 py-1 font-mono text-xs">{d}</li>
						{/each}
					</ul>
				{/if}
			</CardContent>
		</Card>
	</div>
{/if}
