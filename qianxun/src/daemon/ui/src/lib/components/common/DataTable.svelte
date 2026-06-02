<script lang="ts">
	// Stage 7a 通用 DataTable — 简单 key/value 列表 (MVP)
	// 后续 Stage 7c 升级为分页 + 排序 + filter, 现在只支持 1 列表.

	import type { Snippet } from 'svelte';

	type Column = {
		key: string;
		header: string;
		class?: string;
	};

	type Props = {
		columns: Column[];
		rows: Record<string, unknown>[];
		empty?: Snippet;
		testId?: string;
	};

	let { columns, rows, empty, testId }: Props = $props();
</script>

{#if rows.length === 0 && empty}
	{@render empty()}
{:else}
	<table class="w-full caption-bottom text-sm" data-testid={testId}>
		<thead class="border-b">
			<tr>
				{#each columns as col (col.key)}
					<th
						class="text-muted-foreground h-10 px-2 text-left align-middle font-medium"
						class:hidden={false}
					>
						{col.header}
					</th>
				{/each}
			</tr>
		</thead>
		<tbody>
			{#each rows as row, i (i)}
				<tr class="hover:bg-muted/50 border-b transition-colors">
					{#each columns as col (col.key)}
						<td class="p-2 align-middle" class:hidden={false}>
							{row[col.key] ?? ''}
						</td>
					{/each}
				</tr>
			{/each}
		</tbody>
	</table>
{/if}
