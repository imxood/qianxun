<script lang="ts">
	// Stage 7a §3 — Skills 管理面板
	// 列表 / 重载 / 启停 / 详情

	import { onMount } from 'svelte';
	import { RefreshCw, Power, FileText } from '@lucide/svelte';
	import Card from '$lib/components/ui/card/Card.svelte';
	import CardHeader from '$lib/components/ui/card/CardHeader.svelte';
	import CardTitle from '$lib/components/ui/card/CardTitle.svelte';
	import CardDescription from '$lib/components/ui/card/CardDescription.svelte';
	import CardContent from '$lib/components/ui/card/CardContent.svelte';
	import CardFooter from '$lib/components/ui/card/CardFooter.svelte';
	import Button from '$lib/components/ui/button/Button.svelte';
	import Badge from '$lib/components/ui/badge/Badge.svelte';
	import Dialog from '$lib/components/ui/dialog/Dialog.svelte';
	import DialogBody from '$lib/components/ui/dialog/DialogBody.svelte';
	import DialogFooter from '$lib/components/ui/dialog/DialogFooter.svelte';
	import Loading from '$lib/components/common/Loading.svelte';
	import Empty from '$lib/components/common/Empty.svelte';
	import ErrorBanner from '$lib/components/common/ErrorBanner.svelte';
	import PageHeader from '$lib/components/common/PageHeader.svelte';
	import { formatTimestamp, truncate } from '$lib/utils/format';
	import { listSkills, reloadSkills, toggleSkill } from '$lib/api/skills';
	import type { SkillSummary } from '$lib/types/api';

	let skills = $state<SkillSummary[]>([]);
	let loading = $state(true);
	let reloading = $state(false);
	let error = $state<string | null>(null);
	let detailSkill = $state<SkillSummary | null>(null);

	async function refresh() {
		loading = true;
		error = null;
		try {
			skills = await listSkills();
		} catch (e) {
			error = e instanceof Error ? e.message : '加载失败';
		} finally {
			loading = false;
		}
	}

	async function reload() {
		reloading = true;
		error = null;
		try {
			const r = await reloadSkills();
			error = null;
			alert(`已重载 ${r.count} 个 skill`);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '重载失败';
		} finally {
			reloading = false;
		}
	}

	async function toggle(s: SkillSummary) {
		try {
			await toggleSkill(s.name);
			await refresh();
		} catch (e) {
			error = e instanceof Error ? e.message : '切换失败';
		}
	}

	onMount(() => {
		void refresh();
	});
</script>

<PageHeader
	title="Skills"
	description="千寻的 skills 来自 .qianxun/skills/ 目录的 markdown + frontmatter."
>
	{#snippet actions()}
		<Button
			variant="outline"
			size="sm"
			onclick={reload}
			disabled={reloading}
			data-testid="skills-reload"
		>
			<RefreshCw class="h-3.5 w-3.5 {reloading ? 'animate-spin' : ''}" />
			{reloading ? '重载中…' : '重载全部'}
		</Button>
	{/snippet}
</PageHeader>

{#if error}
	<ErrorBanner message={error} class="mb-4" />
{/if}

{#if loading}
	<Loading label="加载 skills…" />
{:else if skills.length === 0}
	<Empty
		title="还没有 skill"
		description="在 .qianxun/skills/ 目录加 markdown 文件 + frontmatter, 然后点「重载」"
	/>
{:else}
	<div class="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3" data-testid="skills-grid">
		{#each skills as s (s.name)}
			<Card>
				<CardHeader>
					<div class="flex items-start justify-between gap-2">
						<div class="flex flex-col gap-1">
							<CardTitle>{s.name}</CardTitle>
							<CardDescription>{truncate(s.description, 100)}</CardDescription>
						</div>
						{#if s.enabled}
							<Badge variant="success">enabled</Badge>
						{:else}
							<Badge variant="outline">disabled</Badge>
						{/if}
					</div>
				</CardHeader>
				<CardContent>
					<p class="text-muted-foreground font-mono text-xs">{s.path}</p>
				</CardContent>
				<CardFooter class="gap-2">
					<Button size="sm" variant="outline" onclick={() => (detailSkill = s)}>
						<FileText class="h-3 w-3" />
						详情
					</Button>
					<div class="ml-auto">
						<Button
							size="sm"
							variant={s.enabled ? 'destructive' : 'outline'}
							onclick={() => toggle(s)}
							data-testid={`skills-toggle-${s.name}`}
						>
							<Power class="h-3 w-3" />
							{s.enabled ? '停用' : '启用'}
						</Button>
					</div>
				</CardFooter>
			</Card>
		{/each}
	</div>
{/if}

<Dialog
	open={detailSkill != null}
	onOpenChange={(v) => {
		if (!v) detailSkill = null;
	}}
	title={detailSkill?.name ?? ''}
	description="Skill manifest"
>
	<DialogBody>
		{#if detailSkill}
			<div class="flex flex-col gap-3 text-sm">
				<div>
					<div class="text-muted-foreground text-xs">描述</div>
					<div>{detailSkill.description}</div>
				</div>
				<div>
					<div class="text-muted-foreground text-xs">路径</div>
					<div class="font-mono text-xs">{detailSkill.path}</div>
				</div>
				<div>
					<div class="text-muted-foreground text-xs">状态</div>
					<div>
						{#if detailSkill.enabled}
							<Badge variant="success">enabled</Badge>
						{:else}
							<Badge variant="outline">disabled</Badge>
						{/if}
					</div>
				</div>
				{#if detailSkill.frontmatter && Object.keys(detailSkill.frontmatter).length > 0}
					<div>
						<div class="text-muted-foreground text-xs">Frontmatter</div>
						<pre
							class="bg-muted overflow-auto rounded p-2 font-mono text-xs">{JSON.stringify(
								detailSkill.frontmatter,
								null,
								2
							)}</pre>
					</div>
				{/if}
				{#if detailSkill.version}
					<div>
						<div class="text-muted-foreground text-xs">版本</div>
						<div>{detailSkill.version}</div>
					</div>
				{/if}
				<p class="text-muted-foreground text-xs">更新于 {formatTimestamp(detailSkill.path)}</p>
			</div>
		{/if}
	</DialogBody>
	<DialogFooter>
		<Button variant="outline" onclick={() => (detailSkill = null)}>关闭</Button>
	</DialogFooter>
</Dialog>
