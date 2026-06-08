<script lang="ts">
	// Stage 7a §3 — MCP Servers 管理面板
	// 列表 / 新增 (stdio/HTTP) / 删除 / 测试连接

	import { onMount } from 'svelte';
	import { Plus, Trash2, Zap, Plug } from '@lucide/svelte';
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
	import { addMcpServer, deleteMcpServer, listMcpServers, testMcpServer } from '$lib/api/mcp';
	import type { McpServerConfig, McpServerSummary, McpTransport } from '$lib/types/api';

	let servers = $state<McpServerSummary[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let dialogOpen = $state(false);
	let draft = $state<McpServerConfig>({
		id: '',
		name: '',
		transport: 'stdio',
		command_or_url: '',
		args: []
	});
	let argsText = $state('');
	let dialogError = $state<string | null>(null);
	let saving = $state(false);

	let testing = $state<Record<string, 'idle' | 'running' | 'ok' | 'fail'>>({});
	let testInfo = $state<Record<string, { tools: number; error?: string } | null>>({});

	async function refresh() {
		loading = true;
		error = null;
		try {
			servers = await listMcpServers();
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
		draft = { id: '', name: '', transport: 'stdio', command_or_url: '', args: [] };
		argsText = '';
		dialogError = null;
		dialogOpen = true;
	}

	async function saveDraft() {
		if (!draft.id.trim() || !draft.name.trim() || !draft.command_or_url.trim()) {
			dialogError = 'ID / name / 命令/URL 都必填';
			return;
		}
		saving = true;
		dialogError = null;
		try {
			const cfg: McpServerConfig = {
				...draft,
				args: argsText
					? argsText
							.split(/\s+/)
							.map((s) => s.trim())
							.filter(Boolean)
					: []
			};
			await addMcpServer(cfg);
			dialogOpen = false;
			await refresh();
		} catch (e) {
			dialogError = e instanceof Error ? e.message : '保存失败';
		} finally {
			saving = false;
		}
	}

	async function remove(id: string) {
		if (!confirm(`确认删除 MCP server "${id}"?`)) return;
		try {
			await deleteMcpServer(id);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '删除失败';
		}
	}

	async function test(id: string) {
		testing = { ...testing, [id]: 'running' };
		testInfo = { ...testInfo, [id]: null };
		try {
			const r = await testMcpServer(id);
			testing = { ...testing, [id]: r.ok ? 'ok' : 'fail' };
			testInfo = {
				...testInfo,
				[id]: { tools: r.tools?.length ?? 0, error: r.error }
			};
		} catch (e) {
			testing = { ...testing, [id]: 'fail' };
			testInfo = {
				...testInfo,
				[id]: { tools: 0, error: e instanceof Error ? e.message : '测试失败' }
			};
		}
	}

	const transportOptions = [
		{ value: 'stdio', label: 'stdio (本地命令)' },
		{ value: 'http', label: 'http (远程 URL)' }
	];
</script>

<PageHeader
	title="MCP Servers"
	description="管理 Model Context Protocol server 配置 (stdio / http)"
>
	{#snippet actions()}
		<Button size="sm" onclick={openCreate} data-testid="mcp-add">
			<Plus class="h-3.5 w-3.5" />
			新增
		</Button>
	{/snippet}
</PageHeader>

{#if error}
	<ErrorBanner message={error} class="mb-4" />
{/if}

{#if loading}
	<Loading label="加载 MCP servers…" />
{:else if servers.length === 0}
	<Empty title="还没有 MCP server" description="点击「新增」添加 stdio / http 类型的 server" />
{:else}
	<div class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3" data-testid="mcp-grid">
		{#each servers as s (s.id)}
			<Card>
				<CardHeader>
					<div class="flex items-start justify-between gap-2">
						<div class="flex flex-col gap-1">
							<CardTitle>{s.name}</CardTitle>
							<CardDescription>
								<span class="font-mono text-xs">{s.id}</span>
							</CardDescription>
						</div>
						<div class="flex flex-col items-end gap-1">
							<Badge variant={s.transport === 'stdio' ? 'info' : 'secondary'}>
								{s.transport}
							</Badge>
							{#if s.connected}
								<Badge variant="success">connected</Badge>
							{:else}
								<Badge variant="outline">disconnected</Badge>
							{/if}
						</div>
					</div>
				</CardHeader>
				<CardContent>
					<p class="text-muted-foreground font-mono text-xs break-all">
						{s.command_or_url}
					</p>
					{#if s.tool_count > 0}
						<p class="text-muted-foreground mt-1 text-xs">
							<Plug class="inline h-3 w-3" /> {s.tool_count} tools
						</p>
					{/if}
					{#if testing[s.id] === 'running'}
						<p class="text-muted-foreground text-xs">测试中…</p>
					{:else if testing[s.id] === 'ok'}
						<p class="text-xs text-green-600 dark:text-green-400">
							ok · {testInfo[s.id]?.tools ?? 0} tools
						</p>
					{:else if testing[s.id] === 'fail'}
						<p class="text-destructive text-xs">
							fail · {testInfo[s.id]?.error ?? 'unknown'}
						</p>
					{/if}
				</CardContent>
				<CardFooter class="gap-2">
					<Button size="sm" variant="outline" onclick={() => test(s.id)}>
						<Zap class="h-3 w-3" />
						测试
					</Button>
					<div class="ml-auto">
						<Button
							size="sm"
							variant="ghost"
							onclick={() => remove(s.id)}
							data-testid={`mcp-delete-${s.id}`}
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
	title="新增 MCP Server"
	description="stdio: 本地命令; http: 远程 URL"
>
	<DialogBody>
		<div class="flex flex-col gap-3">
			<div class="flex flex-col gap-1.5">
				<Label for="mcp-id">ID (唯一)</Label>
				<Input id="mcp-id" bind:value={draft.id} placeholder="例如: filesystem" />
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for="mcp-name">显示名</Label>
				<Input id="mcp-name" bind:value={draft.name} placeholder="例如: 本地文件" />
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for="mcp-transport">Transport</Label>
				<Select id="mcp-transport" options={transportOptions} bind:value={draft.transport} />
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for="mcp-cmd">
					{draft.transport === 'stdio' ? '命令' : 'URL'}
				</Label>
				<Input
					id="mcp-cmd"
					bind:value={draft.command_or_url}
					placeholder={draft.transport === 'stdio' ? 'npx -y @mcp/...' : 'http://localhost:3000'}
				/>
			</div>
			{#if draft.transport === 'stdio'}
				<div class="flex flex-col gap-1.5">
					<Label for="mcp-args">Args (空格分隔)</Label>
					<Input
						id="mcp-args"
						bind:value={argsText}
						placeholder="-y @modelcontextprotocol/server-filesystem /tmp"
					/>
				</div>
			{/if}
			{#if dialogError}
				<ErrorBanner message={dialogError} />
			{/if}
		</div>
	</DialogBody>
	<DialogFooter>
		<Button variant="outline" onclick={() => (dialogOpen = false)} disabled={saving}>
			取消
		</Button>
		<Button onclick={saveDraft} disabled={saving} data-testid="mcp-save">
			{saving ? '保存中…' : '保存'}
		</Button>
	</DialogFooter>
</Dialog>
