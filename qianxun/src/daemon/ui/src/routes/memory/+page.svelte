<script lang="ts">
	// Memory 管理面板 (Stage 7b)
	// 顶部: 搜索框 (调 /v1/memory/search)
	// 左栏: session 列表 (调 /v1/memory/sessions)
	// 右栏: 选中 session 的观察列表 + 详情 + 删除
	import { onMount } from 'svelte';
	import { Search, Trash2, ListTree, X } from '@lucide/svelte';
	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Badge from '$lib/components/ui/badge/Badge.svelte';
	import Input from '$lib/components/ui/input/Input.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import Empty from '$lib/components/common/Empty.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import { formatTimestamp, truncate } from '$lib/utils/format';
	import {
		deleteObservation,
		listMemorySessions,
		listObservations,
		searchMemory
	} from '$lib/api/memory';
	import type { MemoryObservation, MemorySearchResult, MemorySessionSummary } from '$lib/types/api';
	import { t } from '$lib/i18n';
	import { authStore } from '$lib/stores/auth.svelte';

	let sessions = $state<MemorySessionSummary[]>([]);
	let sessionsLoading = $state(true);
	let sessionsError = $state<string | null>(null);

	let selectedSessionId = $state<string | null>(null);
	let observations = $state<MemoryObservation[]>([]);
	let obsLoading = $state(false);
	let obsError = $state<string | null>(null);
	let selectedObs = $state<MemoryObservation | null>(null);

	let searchQuery = $state('');
	let searchResults = $state<MemorySearchResult[]>([]);
	let searching = $state(false);
	let searchError = $state<string | null>(null);
	let searchMode = $state(false);

	async function refreshSessions() {
		sessionsLoading = true;
		sessionsError = null;
		try {
			sessions = await listMemorySessions();
			if (selectedSessionId && !sessions.find((s) => s.id === selectedSessionId)) {
				selectedSessionId = null;
				observations = [];
			}
		} catch (e) {
			sessionsError = e instanceof Error ? e.message : '加载失败';
		} finally {
			sessionsLoading = false;
		}
	}

	async function selectSession(id: string) {
		selectedSessionId = id;
		selectedObs = null;
		searchMode = false;
		obsLoading = true;
		obsError = null;
		try {
			observations = await listObservations(id);
		} catch (e) {
			obsError = e instanceof Error ? e.message : '加载 observation 失败';
		} finally {
			obsLoading = false;
		}
	}

	async function doSearch() {
		const q = searchQuery.trim();
		if (!q) {
			searchResults = [];
			searchMode = false;
			return;
		}
		searchMode = true;
		searching = true;
		searchError = null;
		try {
			const r = await searchMemory({ query: q, limit: 50 });
			searchResults = r.results ?? [];
		} catch (e) {
			searchError = e instanceof Error ? e.message : '搜索失败';
		} finally {
			searching = false;
		}
	}

	function clearSearch() {
		searchQuery = '';
		searchResults = [];
		searchMode = false;
		searchError = null;
	}

	async function removeObservation(id: string) {
		if (!confirm(t('panel.memory.delete_confirm'))) return;
		try {
			await deleteObservation(id);
			if (selectedObs?.id === id) selectedObs = null;
			observations = observations.filter((o) => o.id !== id);
			if (selectedSessionId) {
				// refresh session meta (observation_count)
				await refreshSessions();
				// re-load observations
				await selectSession(selectedSessionId);
			}
		} catch (e) {
			obsError = e instanceof Error ? e.message : '删除失败';
		}
	}

	onMount(() => {
		void refreshSessions();
	});

	// 2026-06-04 fix: 登录后自动重 fetch (见 llm/+page.svelte 注释).
	// firstRun 跳过首次 (onMount 已做), 仅 token 变化时触发.
	let firstRun = true;
	$effect(() => {
		const token = authStore.token;
		if (firstRun) {
			firstRun = false;
			return;
		}
		if (token) {
			void refreshSessions();
		}
	});
</script>

<PageHeader title={t('panel.memory.title')} description={t('panel.memory.desc')}>
	{#snippet actions()}
		<Button variant="outline" size="sm" onclick={refreshSessions}>
			{t('common.refresh')}
		</Button>
	{/snippet}
</PageHeader>

{#if sessionsError}
	<ErrorBanner message={sessionsError} class="mb-4" />
{/if}

<div class="mb-3 flex items-center gap-2">
	<div class="relative flex-1">
		<Search class="text-muted-foreground absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2" />
		<Input
			bind:value={searchQuery}
			placeholder={t('panel.memory.search_placeholder')}
			class="pl-7"
			onkeydown={(e: KeyboardEvent) => {
				if (e.key === 'Enter') void doSearch();
			}}
			data-testid="memory-search-input"
		/>
	</div>
	<Button size="sm" onclick={doSearch} disabled={searching} data-testid="memory-search-btn">
		{searching ? t('common.loading') : t('common.search')}
	</Button>
	{#if searchMode}
		<Button size="sm" variant="outline" onclick={clearSearch}>
			<X class="h-3 w-3" />
			{t('common.cancel')}
		</Button>
	{/if}
</div>

{#if searchError}
	<ErrorBanner message={searchError} class="mb-3" />
{/if}

{#if searchMode}
	<Card class="mb-3" data-testid="memory-search-results">
		<CardHeader>
			<CardTitle>{t('panel.memory.search_results')}</CardTitle>
		</CardHeader>
		<CardContent>
			{#if searching}
				<Loading label={t('common.loading')} />
			{:else if searchResults.length === 0}
				<Empty title={t('panel.memory.search_empty')} />
			{:else}
				<ul class="flex flex-col gap-1.5 text-sm">
					{#each searchResults as r (r.id)}
						<li class="border-border hover:bg-muted/50 rounded border p-2">
							<div class="text-muted-foreground text-xs">
								<span class="font-mono">{r.session_id}</span> · {r.created_at ?? '—'}
								{#if r.score != null}
									<Badge variant="info">score {r.score.toFixed(2)}</Badge>
								{/if}
							</div>
							<div class="mt-1">{truncate(r.content, 300)}</div>
						</li>
					{/each}
				</ul>
			{/if}
		</CardContent>
	</Card>
{/if}

<div class="grid grid-cols-1 gap-3 md:grid-cols-3" data-testid="memory-layout">
	<!-- 左栏: session 列表 -->
	<Card class="md:col-span-1">
		<CardHeader>
			<CardTitle>
				<ListTree class="mr-1 inline h-3.5 w-3.5" />
				Sessions
			</CardTitle>
		</CardHeader>
		<CardContent>
			{#if sessionsLoading}
				<Loading label={t('common.loading')} />
			{:else if sessions.length === 0}
				<Empty title={t('panel.memory.empty_sessions')} description={t('panel.memory.empty_sessions_desc')} />
			{:else}
				<ul class="flex flex-col gap-1">
					{#each sessions as s (s.id)}
						{@const active = s.id === selectedSessionId}
						<li>
							<button
								type="button"
								class="w-full rounded-md border p-2 text-left text-xs transition-colors"
								class:bg-accent={active}
								class:border-accent={active}
								class:border-border={!active}
								onclick={() => selectSession(s.id)}
								data-testid={`memory-session-${s.id}`}
							>
								<div class="font-mono">{truncate(s.id, 20)}</div>
								<div class="text-muted-foreground mt-0.5">
									{s.observation_count} obs · {formatTimestamp(s.last_active ?? s.created_at)}
								</div>
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</CardContent>
	</Card>

	<!-- 右栏: observation 列表 + 详情 -->
	<div class="flex flex-col gap-3 md:col-span-2">
		{#if obsError}
			<ErrorBanner message={obsError} />
		{/if}

		<Card data-testid="memory-observations-card">
			<CardHeader>
				<CardTitle>
					{t('panel.memory.observations')}
					{#if selectedSessionId}
						<span class="text-muted-foreground ml-2 font-mono text-xs">
							{selectedSessionId}
						</span>
					{/if}
				</CardTitle>
			</CardHeader>
			<CardContent>
				{#if !selectedSessionId}
					<Empty title={t('common.empty')} />
				{:else if obsLoading}
					<Loading label={t('common.loading')} />
				{:else if observations.length === 0}
					<Empty title={t('panel.memory.empty_observations')} />
				{:else}
					<ul class="flex flex-col gap-1.5">
						{#each observations as o (o.id)}
							{@const sel = o.id === selectedObs?.id}
							<li>
								<button
									type="button"
									class="w-full rounded-md border p-2 text-left text-sm transition-colors"
									class:bg-accent={sel}
									class:border-accent={sel}
									class:border-border={!sel}
									onclick={() => (selectedObs = o)}
									data-testid={`memory-obs-${o.id}`}
								>
									<div class="text-muted-foreground text-xs">
										<span class="font-mono">{truncate(o.id, 12)}</span> ·
										{formatTimestamp(o.timestamp)}
									</div>
									<div class="mt-1">{truncate(o.data, 200)}</div>
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</CardContent>
		</Card>

		{#if selectedObs}
			<Card data-testid="memory-obs-detail">
				<CardHeader>
					<CardTitle>{t('common.detail')}</CardTitle>
				</CardHeader>
				<CardContent>
					<dl class="grid grid-cols-1 gap-2 text-sm md:grid-cols-2">
						<div>
							<dt class="text-muted-foreground text-xs">ID</dt>
							<dd class="font-mono text-xs break-all">{selectedObs.id}</dd>
						</div>
						<div>
							<dt class="text-muted-foreground text-xs">Session</dt>
							<dd class="font-mono text-xs break-all">{selectedObs.session_id}</dd>
						</div>
						<div>
							<dt class="text-muted-foreground text-xs">Timestamp</dt>
							<dd>{formatTimestamp(selectedObs.timestamp)}</dd>
						</div>
						<div>
							<dt class="text-muted-foreground text-xs">Created</dt>
							<dd>{selectedObs.created_at}</dd>
						</div>
						<div class="md:col-span-2">
							<dt class="text-muted-foreground text-xs">Data (JSON)</dt>
							<dd class="bg-muted mt-1 rounded p-2 text-sm whitespace-pre-wrap font-mono">
								{selectedObs.data}
							</dd>
						</div>
					</dl>
					<div class="mt-3 flex justify-end">
						<Button
							size="sm"
							variant="destructive"
							onclick={() => removeObservation(selectedObs!.id)}
							data-testid="memory-obs-delete"
						>
							<Trash2 class="h-3 w-3" />
							{t('common.delete')}
						</Button>
					</div>
				</CardContent>
			</Card>
		{/if}
	</div>
</div>
