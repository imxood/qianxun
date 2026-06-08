<script lang="ts">
	// Stage 7a §3 — LLM Provider 管理面板
	// 列表 / 新增 / 编辑 / 删除 / 切 active / 测试连接

	import { onMount } from 'svelte';
	import { Plus, RefreshCw, Trash2, Pencil, Zap, CheckCircle2 } from '@lucide/svelte';
	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardDescription from '$lib/components/ui/card/CardDescription.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import CardFooter from '$lib/components/ui/card/CardFooter.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Badge from '$lib/components/ui/badge/Badge.svelte';
	import Input from '$lib/components/ui/input/Input.svelte';
	import Label from '$lib/components/ui/label/Label.svelte';
	import Select from '$lib/components/ui/select/Select.svelte';
	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import DialogBody from '$lib/components/ui/dialog/DialogBody.svelte';
	import DialogFooter from '$lib/components/ui/dialog/DialogFooter.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import Empty from '$lib/components/common/Empty.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import { formatLatency } from '$lib/utils/format';
	import {
		activateProvider,
		createProvider,
		deleteProvider,
		listProviders,
		testProvider,
		updateProvider
	} from '$lib/api/llm';
	import type { LlmProviderConfig, LlmProviderSummary } from '$lib/types/api';

	const PROVIDER_OPTIONS = [
		{ value: 'deepseek', label: 'DeepSeek (Anthropic compat)' },
		{ value: 'anthropic', label: 'Anthropic' },
		{ value: 'minimax', label: 'minimax' },
		{ value: 'openai', label: 'OpenAI' },
		{ value: 'custom', label: 'Custom (OpenAI compat)' }
	];

	let providers = $state<LlmProviderSummary[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let dialogOpen = $state(false);
	let dialogMode = $state<'create' | 'edit'>('create');
	let draft = $state<LlmProviderConfig>({
		id: '',
		provider: 'deepseek',
		model: 'deepseek-v4-flash',
		base_url: '',
		api_key: ''
	});
	let dialogError = $state<string | null>(null);
	let saving = $state(false);

	let testing = $state<Record<string, 'idle' | 'running' | 'ok' | 'fail'>>({});
	let testLatency = $state<Record<string, number | null>>({});
	let testError = $state<Record<string, string | null>>({});

	async function refresh() {
		loading = true;
		error = null;
		try {
			providers = await listProviders();
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void refresh();
	});

	function openCreate() {
		draft = {
			id: '',
			provider: 'deepseek',
			model: 'deepseek-v4-flash',
			base_url: '',
			api_key: ''
		};
		dialogMode = 'create';
		dialogError = null;
		dialogOpen = true;
	}

	function openEdit(p: LlmProviderSummary) {
		draft = {
			id: p.id,
			provider: p.provider,
			model: p.model,
			base_url: p.base_url ?? '',
			api_key: ''
		};
		dialogMode = 'edit';
		dialogError = null;
		dialogOpen = true;
	}

	async function saveDraft() {
		if (!draft.id.trim()) {
			dialogError = 'ID 不能为空';
			return;
		}
		saving = true;
		dialogError = null;
		try {
			if (dialogMode === 'create') {
				await createProvider(draft);
			} else {
				await updateProvider(draft.id, draft);
			}
			dialogOpen = false;
			await refresh();
		} catch (e) {
			dialogError = e instanceof Error ? e.message : '保存失败';
		} finally {
			saving = false;
		}
	}

	async function remove(id: string) {
		if (!confirm(`确认删除 provider "${id}" ?`)) return;
		try {
			await deleteProvider(id);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '删除失败';
		}
	}

	async function activate(id: string) {
		try {
			await activateProvider(id);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '切换失败';
		}
	}

	async function test(id: string) {
		testing = { ...testing, [id]: 'running' };
		testError = { ...testError, [id]: null };
		try {
			const r = await testProvider(id);
			testing = { ...testing, [id]: r.ok ? 'ok' : 'fail' };
			testLatency = { ...testLatency, [id]: r.latency_ms ?? null };
			testError = { ...testError, [id]: r.error ?? null };
		} catch (e) {
			testing = { ...testing, [id]: 'fail' };
			testError = { ...testError, [id]: e instanceof Error ? e.message : '测试失败' };
		}
	}
</script>

<PageHeader
	title="LLM Providers"
	description="管理千寻 daemon 的 LLM 模型接入 (添加 / 编辑 / 切换 / 测试)"
>
	{#snippet actions()}
		<Button variant="outline" size="sm" onclick={refresh} data-testid="llm-refresh">
			<RefreshCw class="h-3.5 w-3.5" />
			刷新
		</Button>
		<Button size="sm" onclick={openCreate} data-testid="llm-add">
			<Plus class="h-3.5 w-3.5" />
			新增
		</Button>
	{/snippet}
</PageHeader>

{#if error}
	<ErrorBanner message={error} class="mb-4" />
{/if}

{#if loading}
	<Loading label="加载 providers…" />
{:else if providers.length === 0}
	<Empty title="还没有 LLM provider" description="点击右上角「新增」开始配置" />
{:else}
	<div class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3" data-testid="llm-grid">
		{#each providers as p (p.id)}
			<Card>
				<CardHeader>
					<div class="flex items-start justify-between gap-2">
						<div class="flex flex-col gap-1">
							<CardTitle>{p.id}</CardTitle>
							<CardDescription>
								<span class="font-mono">{p.provider}</span>
								·
								<span class="font-mono">{p.model}</span>
							</CardDescription>
						</div>
						<div class="flex flex-col items-end gap-1">
							{#if p.active}
								<Badge variant="success">ACTIVE</Badge>
							{:else}
								<Badge variant="outline">inactive</Badge>
							{/if}
							{#if p.has_key}
								<Badge variant="info">key ✓</Badge>
							{:else}
								<Badge variant="warning">no key</Badge>
							{/if}
						</div>
					</div>
				</CardHeader>
				<CardContent>
					{#if testing[p.id] === 'running'}
						<p class="text-muted-foreground text-xs">测试中…</p>
					{:else if testing[p.id] === 'ok'}
						<p class="text-xs text-green-600 dark:text-green-400">
							<CheckCircle2 class="inline h-3 w-3" />
							ok · {formatLatency(testLatency[p.id] ?? null)}
						</p>
					{:else if testing[p.id] === 'fail'}
						<p class="text-destructive text-xs">
							fail · {testError[p.id] ?? 'unknown'}
						</p>
					{:else}
						<p class="text-muted-foreground text-xs">未测试</p>
					{/if}
				</CardContent>
				<CardFooter class="gap-2">
					{#if !p.active}
						<Button size="sm" variant="outline" onclick={() => activate(p.id)}>激活</Button>
					{/if}
					<Button size="sm" variant="outline" onclick={() => test(p.id)}>
						<Zap class="h-3 w-3" />
						测试
					</Button>
					<div class="ml-auto flex gap-1">
						<Button size="sm" variant="ghost" onclick={() => openEdit(p)} title="编辑">
							<Pencil class="h-3 w-3" />
						</Button>
						<Button
							size="sm"
							variant="ghost"
							onclick={() => remove(p.id)}
							title="删除"
							data-testid={`llm-delete-${p.id}`}
						>
							<Trash2 class="h-3 w-3" />
						</Button>
					</div>
				</CardFooter>
			</Card>
		{/each}
	</div>
{/if}

<Dialog
	open={dialogOpen}
	onOpenChange={(v) => (dialogOpen = v)}
	title={dialogMode === 'create' ? '新增 Provider' : `编辑 Provider: ${draft.id}`}
	description="API key 仅运行时使用, 不会持久化到配置文件."
>
	<DialogBody>
		<div class="flex flex-col gap-3">
			<div class="flex flex-col gap-1.5">
				<Label for="prov-id">ID (唯一标识)</Label>
				<Input id="prov-id" bind:value={draft.id} disabled={dialogMode === 'edit'} />
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for="prov-provider">Provider 类型</Label>
				<Select
					id="prov-provider"
					options={PROVIDER_OPTIONS}
					bind:value={draft.provider}
				/>
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for="prov-model">Model</Label>
				<Input id="prov-model" bind:value={draft.model} />
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for="prov-url">Base URL (可选)</Label>
				<Input
					id="prov-url"
					bind:value={draft.base_url}
					placeholder="https://api.deepseek.com/anthropic"
				/>
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for="prov-key">API Key</Label>
				<Input
					id="prov-key"
					type="password"
					bind:value={draft.api_key}
					placeholder={dialogMode === 'edit' ? '留空 = 不修改' : 'sk-...'}
				/>
			</div>
			{#if dialogError}
				<ErrorBanner message={dialogError} />
			{/if}
		</div>
	</DialogBody>
	<DialogFooter>
		<Button variant="outline" onclick={() => (dialogOpen = false)} disabled={saving}>
			取消
		</Button>
		<Button onclick={saveDraft} disabled={saving} data-testid="llm-save">
			{saving ? '保存中…' : '保存'}
		</Button>
	</DialogFooter>
</Dialog>
