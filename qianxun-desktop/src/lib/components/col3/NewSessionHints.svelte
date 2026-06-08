<script lang="ts">
	import Icon from '../shared/Icon.svelte';
</script>

<div class="p-3 space-y-4">
	<div>
		<div class="text-[11px] text-zinc-700 dark:text-zinc-400 font-medium mb-1.5">如何归类?</div>
		<ul class="text-[11px] text-zinc-500 space-y-1.5 leading-relaxed">
			<li>· 选了文件夹 → 落入对应项目</li>
			<li>· 不选 (未选) → 落入 Chat</li>
			<li>· 之后在侧栏可拖动归类</li>
		</ul>
	</div>
	<div>
		<div class="text-[11px] text-zinc-700 dark:text-zinc-400 font-medium mb-1.5">主 Agent 能做什么?</div>
		<ul class="text-[11px] text-zinc-500 space-y-1.5 leading-relaxed">
			<li>· 直接回答 / 写代码</li>
			<li>· 拆 Plan 拉子 Agent 干活</li>
			<li>· 沉淀项目经验 / 会话纪要</li>
		</ul>
	</div>
	<div>
		<div class="text-[11px] text-zinc-700 dark:text-zinc-400 font-medium mb-1.5">快捷键</div>
		<div class="text-[11px] text-zinc-500 space-y-1">
			<div><kbd class="px-1 py-0.5 bg-zinc-100 dark:bg-zinc-800 rounded text-zinc-700 dark:text-zinc-300 font-mono text-[10px]">⌘ N</kbd> 新建</div>
			<div><kbd class="px-1 py-0.5 bg-zinc-100 dark:bg-zinc-800 rounded text-zinc-700 dark:text-zinc-300 font-mono text-[10px]">⌘ K</kbd> 命令面板</div>
			<div><kbd class="px-1 py-0.5 bg-zinc-100 dark:bg-zinc-800 rounded text-zinc-700 dark:text-zinc-300 font-mono text-[10px]">⌘ /</kbd> 切换主题</div>
		</div>
	</div>
</div>
