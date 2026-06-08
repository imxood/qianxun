<script lang="ts">
	// Chat Sessions 管理面板 (Stage 7b)
	// 列表 / 过滤 (active/paused/all) / 行操作 (pause / cancel / delete / 详情)

	import { onMount } from 'svelte';
	import { Eye, Pause, Ban, Trash2, RefreshCw } from '@lucide/svelte';
	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Badge from '$lib/components/ui/badge/Badge.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import Empty from '$lib/components/common/Empty.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import DialogBody from '$lib/components/ui/dialog/DialogBody.svelte';
	import DialogFooter from '$lib/components/ui/dialog/DialogFooter.svelte';
	import { formatTimestamp } from '$lib/utils/format';
	import {
		cancelChatSession,
		deleteChatSession,
		getChatSession,
		listChatSessions,
		pauseChatSession
	} from '$lib/api/sessions';
	import type {
		ChatSessionDetail,
		ChatSessionSummary,
		SessionStatus
	} from '$lib/types/api';
	import { t } from '$lib/i18n';

	type Filter = SessionStatus | 'all';
	let filter = $state<Filter>('all');
	let sessions = $state<ChatSessionSummary[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let detailId = $state<string | null>(null);
	let detail = $state<ChatSessionDetail | null>(null);
	let detailLoading = $state(false);
	let detailError = $state<string | null>(null);

	let acting = $state<Record<string, 'cancel' | 'pause' | 'delete' | null>>({});

	async function refresh() {
		loading = true;
		error = null;
		try {
			const r = await listChatSessions({ status: filter });
			sessions = r.sessions;
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		} finally {
			loading = false;
		}
	}

	async function viewDetail(id: string) {
		detailId = id;
		detail = null;
		detailError = null;
		detailLoading = true;
		try {
			detail = await getChatSession(id);
		} catch (e) {
			detailError = e instanceof Error ? e.message : '加载详情失败';
		} finally {
			detailLoading = false;
		}
	}

	async function doAction(s: ChatSessionSummary, action: 'cancel' | 'pause' | 'delete') {
		acting = { ...acting, [s.id]: action };
		try {
			if (action === 'cancel') await cancelChatSession(s.id);
			else if (action === 'pause') await pauseChatSession(s.id);
			else await deleteChatSession(s.id);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : `${action} 失败`;
		} finally {
			acting = { ...acting, [s.id]: null };
		}
	}

	function statusVariant(s: SessionStatus): 'success' | 'warning' | 'outline' | 'destructive' {
		if (s === 'active') return 'success';
		if (s === 'paused') return 'warning';
		if (s === 'cancelled') return 'destructive';
		return 'outline';
	}

	onMount(() => {
		void refresh();
	});
</script>

<PageHeader title={t('panel.sessions.title')} description={t('panel.sessions.desc')}>
	{#snippet actions()}
		<Button variant="outline" size="sm" onclick={refresh}>
			<RefreshCw class="h-3.5 w-3.5" />
			{t('common.refresh')}
		</Button>
	{/snippet}
</PageHeader>

{#if error}
	<ErrorBanner message={error} class="mb-4" />
{/if}

<div class="mb-3 flex items-center gap-2 text-sm">
	<span class="text-muted-foreground">Filter:</span>
	{#each [{ k: 'all', l: t('panel.sessions.filter_all') }, { k: 'active', l: t('panel.sessions.filter_active') }, { k: 'paused', l: t('panel.sessions.filter_paused') }] as opt (opt.k)}
		<Button
			size="sm"
			variant={filter === opt.k ? 'default' : 'outline'}
			onclick={() => {
				filter = opt.k as Filter;
				void refresh();
			}}
			data-testid={`sessions-filter-${opt.k}`}
		>
			{opt.l}
		</Button>
	{/each}
</div>

{#if loading}
	<Loading label={t('common.loading')} />
{:else if sessions.length === 0}
	<Empty title={t('panel.sessions.empty')} />
{:else}
	<Card>
		<CardContent class="p-0">
			<div class="overflow-x-auto">
				<table class="w-full text-sm" data-testid="sessions-table">
					<thead class="bg-muted text-muted-foreground border-b text-left">
						<tr>
							<th class="px-3 py-2 font-medium">ID</th>
							<th class="px-3 py-2 font-medium">Model</th>
							<th class="px-3 py-2 font-medium">Status</th>
							<th class="px-3 py-2 font-medium">Created</th>
							<th class="px-3 py-2 font-medium">Last Active</th>
							<th class="px-3 py-2 text-right font-medium">Msgs</th>
							<th class="px-3 py-2 text-right font-medium">Tokens</th>
							<th class="px-3 py-2 text-right font-medium">Action</th>
						</tr>
					</thead>
					<tbody>
						{#each sessions as s (s.id)}
							<tr class="border-b" data-testid={`sessions-row-${s.id}`}>
								<td class="px-3 py-2 font-mono text-xs">{s.id.slice(0, 12)}</td>
								<td class="px-3 py-2 font-mono text-xs">{s.model}</td>
								<td class="px-3 py-2">
									<Badge variant={statusVariant(s.status)}>
										{t('panel.sessions.status_' + s.status)}
									</Badge>
								</td>
								<td class="px-3 py-2 text-xs">{formatTimestamp(s.created_at)}</td>
								<td class="px-3 py-2 text-xs">{formatTimestamp(s.last_active)}</td>
								<td class="px-3 py-2 text-right">{s.message_count}</td>
								<td class="px-3 py-2 text-right font-mono text-xs">
									{s.token_usage.total.toLocaleString()}
								</td>
								<td class="px-3 py-2">
									<div class="flex items-center justify-end gap-1">
										<Button
											size="sm"
											variant="ghost"
											onclick={() => viewDetail(s.id)}
											title={t('common.detail')}
										>
											<Eye class="h-3 w-3" />
										</Button>
										{#if s.status === 'active'}
											<Button
												size="sm"
												variant="ghost"
												disabled={acting[s.id] != null}
												onclick={() => doAction(s, 'pause')}
												title={t('panel.sessions.pause')}
												data-testid={`sessions-pause-${s.id}`}
											>
												<Pause class="h-3 w-3" />
											</Button>
											<Button
												size="sm"
												variant="ghost"
												disabled={acting[s.id] != null}
												onclick={() => doAction(s, 'cancel')}
												title={t('panel.sessions.cancel')}
												data-testid={`sessions-cancel-${s.id}`}
											>
												<Ban class="h-3 w-3" />
											</Button>
										{/if}
										<Button
											size="sm"
											variant="ghost"
											disabled={acting[s.id] != null}
											onclick={() => doAction(s, 'delete')}
											title={t('common.delete')}
											data-testid={`sessions-delete-${s.id}`}
										>
											<Trash2 class="h-3 w-3" />
										</Button>
									</div>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		</CardContent>
	</Card>
{/if}

<Dialog
	open={detailId != null}
	onOpenChange={(v) => {
		if (!v) {
			detailId = null;
			detail = null;
		}
	}}
	title={t('panel.sessions.detail_title') + ': ' + (detailId ?? '')}
	description={t('panel.sessions.event_log')}
>
	<DialogBody>
		{#if detailLoading}
			<Loading label={t('common.loading')} />
		{:else if detailError}
			<ErrorBanner message={detailError} />
		{:else if detail}
			<div class="flex flex-col gap-3 text-sm">
				<div class="text-muted-foreground flex flex-wrap gap-x-4 gap-y-1 text-xs">
					<span>Model: <span class="font-mono">{detail.model}</span></span>
					<span>Status: <Badge variant={statusVariant(detail.status)}>{detail.status}</Badge></span>
					<span>Created: {formatTimestamp(detail.created_at)}</span>
					<span>Last: {formatTimestamp(detail.last_active)}</span>
					<span>Tokens: {detail.token_usage.total.toLocaleString()}</span>
				</div>
				<pre
					class="bg-muted max-h-96 overflow-auto rounded p-2 font-mono text-xs whitespace-pre-wrap"
					data-testid="sessions-event-log">{JSON.stringify(detail.messages ?? [], null, 2)}</pre>
			</div>
		{/if}
	</DialogBody>
	<DialogFooter>
		<Button variant="outline" onclick={() => (detailId = null)}>{t('common.close')}</Button>
	</DialogFooter>
</Dialog>
