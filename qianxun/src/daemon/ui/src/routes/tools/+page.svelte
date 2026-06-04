<script lang="ts">
	// Stage 7a §3 — Tools 管理面板
	// 列表 / 详情 (schema) / 试用 (invoke, JSON 输入)

	import { onMount } from 'svelte';
	import { Wrench, Play, Code2 } from '@lucide/svelte';
	import { authStore } from '$lib/stores/auth.svelte';
	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardDescription from '$lib/components/ui/card/CardDescription.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import CardFooter from '$lib/components/ui/card/CardFooter.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Badge from '$lib/components/ui/badge/Badge.svelte';
	import Textarea from '$lib/components/ui/input/Textarea.svelte';
	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import DialogBody from '$lib/components/ui/dialog/DialogBody.svelte';
	import DialogFooter from '$lib/components/ui/dialog/DialogFooter.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import Empty from '$lib/components/common/Empty.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import { formatLatency } from '$lib/utils/format';
	import { invokeTool, listTools } from '$lib/api/tools';
	import type { ToolDefinition, ToolInvokeResult } from '$lib/types/api';

	let tools = $state<ToolDefinition[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let invokingTool = $state<ToolDefinition | null>(null);
	let argumentsJson = $state('{}');
	let invokeResult = $state<ToolInvokeResult | null>(null);
	let invokeError = $state<string | null>(null);
	let invoking = $state(false);

	async function refresh() {
		loading = true;
		error = null;
		try {
			tools = await listTools();
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

	function openInvoke(t: ToolDefinition) {
		invokingTool = t;
		// 尝试根据 schema 生成一个默认 JSON 模板
		argumentsJson = JSON.stringify(generateTemplate(t.input_schema), null, 2);
		invokeResult = null;
		invokeError = null;
	}

	function generateTemplate(schema: Record<string, unknown>): Record<string, unknown> {
		const props = (schema.properties as Record<string, unknown> | undefined) ?? {};
		const required = (schema.required as string[] | undefined) ?? [];
		const tpl: Record<string, unknown> = {};
		for (const [k, v] of Object.entries(props)) {
			const sv = v as Record<string, unknown>;
			const type = sv.type as string | undefined;
			if (type === 'string') tpl[k] = required.includes(k) ? '' : '';
			else if (type === 'number' || type === 'integer') tpl[k] = 0;
			else if (type === 'boolean') tpl[k] = false;
			else if (type === 'array') tpl[k] = [];
			else if (type === 'object') tpl[k] = {};
		}
		return tpl;
	}

	async function doInvoke() {
		if (!invokingTool) return;
		let args: Record<string, unknown>;
		try {
			args = JSON.parse(argumentsJson);
		} catch (e) {
			invokeError = `JSON 解析失败: ${e instanceof Error ? e.message : String(e)}`;
			return;
		}
		invoking = true;
		invokeError = null;
		invokeResult = null;
		try {
			invokeResult = await invokeTool(invokingTool.name, args);
		} catch (e) {
			invokeError = e instanceof Error ? e.message : '调用失败';
		} finally {
			invoking = false;
		}
	}
</script>

<PageHeader
	title="Tools"
	description="查看 / 试用千寻的内置工具 (不走 LLM, 直接调 ToolRegistry)"
>
	{#snippet actions()}
		<Badge variant="outline">共 {tools.length} 个</Badge>
	{/snippet}
</PageHeader>

{#if error}
	<ErrorBanner message={error} class="mb-4" />
{/if}

{#if loading}
	<Loading label="加载 tools…" />
{:else if tools.length === 0}
	<Empty title="还没有 tool" description="Tool registry 暂未注册任何 tool" />
{:else}
	<div class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3" data-testid="tools-grid">
		{#each tools as t (t.name)}
			<Card>
				<CardHeader>
					<div class="flex items-start gap-2">
						<Wrench class="text-muted-foreground mt-0.5 h-4 w-4 shrink-0" />
						<div class="flex flex-col gap-1">
							<CardTitle class="font-mono">{t.name}</CardTitle>
							<CardDescription>{t.description}</CardDescription>
						</div>
					</div>
				</CardHeader>
				<CardContent>
					<details class="text-xs">
						<summary
							class="text-muted-foreground hover:text-foreground cursor-pointer select-none"
						>
							<Code2 class="inline h-3 w-3" /> Schema
						</summary>
						<pre
							class="bg-muted mt-2 overflow-auto rounded p-2 font-mono text-[10px]">{JSON.stringify(
								t.input_schema,
								null,
								2
							)}</pre>
					</details>
				</CardContent>
				<CardFooter>
					<Button
						size="sm"
						variant="outline"
						onclick={() => openInvoke(t)}
						data-testid={`tools-invoke-${t.name}`}
					>
						<Play class="h-3 w-3" />
						试用
					</Button>
				</CardFooter>
			</Card>
		{/each}
	</div>
{/if}

<Dialog
	open={invokingTool != null}
	onOpenChange={(v) => {
		if (!v) {
			invokingTool = null;
			invokeResult = null;
			invokeError = null;
		}
	}}
	title={invokingTool ? `试用: ${invokingTool.name}` : ''}
	description={invokingTool?.description ?? ''}
	class="max-w-2xl"
>
	<DialogBody>
		{#if invokingTool}
			<div class="flex flex-col gap-3">
				<div class="flex flex-col gap-1.5">
					<label for="args" class="text-sm font-medium">Arguments (JSON)</label>
					<Textarea
						id="args"
						bind:value={argumentsJson}
						class="font-mono text-xs"
						rows={8}
					/>
				</div>
				{#if invokeError}
					<ErrorBanner message={invokeError} />
				{/if}
				{#if invokeResult}
					<div class="bg-muted rounded p-3 text-xs">
						<div class="mb-1 flex items-center gap-2 font-semibold">
							<span>Output</span>
							{#if invokeResult.elapsed_ms != null}
								<Badge variant="info">{formatLatency(invokeResult.elapsed_ms)}</Badge>
							{/if}
						</div>
						<pre class="overflow-auto whitespace-pre-wrap">{invokeResult.output}</pre>
					</div>
				{/if}
			</div>
		{/if}
	</DialogBody>
	<DialogFooter>
		<Button variant="outline" onclick={() => (invokingTool = null)}>关闭</Button>
		<Button onclick={doInvoke} disabled={invoking} data-testid="tools-run">
			{invoking ? '调用中…' : '调用'}
		</Button>
	</DialogFooter>
</Dialog>
