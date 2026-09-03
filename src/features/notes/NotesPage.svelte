<script lang="ts">
  /**
   * 笔记页（M5）：列表 | CodeMirror 编辑 | Markdown 预览。
   * 纯文件库（ADR-006）；保存走原子写；删除入 .trash 可救。
   */
  import { onMount } from 'svelte';
  import { EditorView, basicSetup } from 'codemirror';
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
  import { languages } from '@codemirror/language-data';
  import { marked } from 'marked';
  import { call } from '../../lib/ipc';
  import type { NoteContent, NoteMeta } from '../../lib/ipc/contract';
  import { settings } from '../../stores/settings.svelte';
  import { harness } from '../../stores/harness.svelte';

  let notes = $state<NoteMeta[]>([]);
  let filter = $state('');
  let activePath = $state<string | null>(null);
  let activeNote = $state<NoteContent | null>(null);
  let dirty = $state(false);
  let saving = $state(false);
  let errorText = $state('');
  let previewing = $state(false);
  let host: HTMLDivElement | null = $state(null);
  let view: EditorView | null = null;

  // ---- AI 整理（M6：经 qx-bridge 的 /qx/notes/organize） ----
  let organizing = $state(false);
  let organizeOpen = $state(false);
  let organizeInstruction = $state('');
  let organizeResult = $state('');
  let organizeError = $state('');
  const dshOrigin = $derived(harness.status.phase === 'ready' ? harness.status.origin : '');

  const vault = $derived(settings.current?.notes.vaultDir ?? '');
  const filtered = $derived.by(() => {
    const keyword = filter.trim().toLowerCase();
    if (!keyword) return notes;
    return notes.filter(
      (note) =>
        note.title.toLowerCase().includes(keyword) ||
        note.path.toLowerCase().includes(keyword) ||
        note.tags.some((tag) => tag.toLowerCase().includes(keyword)),
    );
  });
  const previewHtml = $derived(
    activeNote ? (marked.parse(activeNote.body, { async: false }) as string) : '',
  );

  onMount(() => {
    view = new EditorView({
      extensions: [
        basicSetup,
        markdown({ codeLanguages: languages, base: markdownLanguage }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && activeNote) {
            activeNote = { ...activeNote, body: update.state.doc.toString() };
            dirty = true;
          }
        }),
      ],
      parent: host!,
    });
    return () => view?.destroy();
  });

  // 库目录就绪（或首次初始化）后拉清单。
  $effect(() => {
    if (vault) void refresh();
  });

  async function initVault(): Promise<void> {
    errorText = '';
    try {
      const dir = await call<string>('notes_init', { vault: null });
      await settings.update({ notes: { vaultDir: dir } });
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function refresh(): Promise<void> {
    try {
      notes = await call<NoteMeta[]>('notes_list', { vault });
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function open(note: NoteMeta): Promise<void> {
    if (dirty && !window.confirm('当前笔记未保存，切换将丢弃修改。继续？')) return;
    errorText = '';
    try {
      activeNote = await call<NoteContent>('notes_read', { vault, path: note.path });
      activePath = note.path;
      dirty = false;
      view?.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: activeNote.body },
      });
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function save(): Promise<void> {
    if (!activeNote || saving) return;
    saving = true;
    errorText = '';
    try {
      // 正文含 frontmatter（由编辑器同一文档维护）。
      const content = view?.state.doc.toString() ?? activeNote.body;
      const meta = await call<NoteMeta>('notes_save', {
        vault,
        path: activeNote.meta.path,
        content,
      });
      activeNote = { ...activeNote, meta, body: content };
      dirty = false;
      await refresh();
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    } finally {
      saving = false;
    }
  }

  async function create(): Promise<void> {
    const title = window.prompt('笔记标题');
    if (!title) return;
    errorText = '';
    try {
      const meta = await call<NoteMeta>('notes_create', { vault, title });
      await refresh();
      await open(meta);
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function remove(): Promise<void> {
    if (!activeNote) return;
    if (!window.confirm(`删除「${activeNote.meta.title}」？（移入 .trash 可找回）`)) return;
    errorText = '';
    try {
      await call('notes_delete', { vault, path: activeNote.meta.path });
      activeNote = null;
      activePath = null;
      await refresh();
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  function formatDate(stamp: number): string {
    const date = new Date(stamp);
    return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(
      date.getDate(),
    ).padStart(2, '0')}`;
  }

  async function runOrganize(): Promise<void> {
    if (!organizeInstruction.trim()) {
      organizeError = '先写整理指令（例如：把重复的条目合并成一篇清单）';
      return;
    }
    organizing = true;
    organizeError = '';
    organizeResult = '';
    try {
      const response = await fetch(`${dshOrigin}/qx/notes/organize`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          instruction: organizeInstruction.trim(),
          // 未选 = 全库整理；结果由模型自行取舍。
          paths: null,
        }),
      });
      const payload = (await response.json()) as { ok: boolean; result?: string; error?: string };
      if (!payload.ok) throw new Error(payload.error ?? `HTTP ${response.status}`);
      organizeResult = payload.result ?? '';
    } catch (error) {
      organizeError = error instanceof Error ? error.message : String(error);
    } finally {
      organizing = false;
    }
  }

  async function saveOrganizeAsNote(): Promise<void> {
    if (!organizeResult.trim()) return;
    const title = window.prompt('保存为笔记，标题：', 'AI 整理稿');
    if (!title) return;
    try {
      const today = new Date().toISOString().slice(0, 10);
      const content = `---\ntitle: ${title}\ntags: [ai-整理]\ncreated: ${today}\n---\n\n${organizeResult}`;
      const meta = await call<NoteMeta>('notes_create', { vault, title });
      await call<NoteMeta>('notes_save', { vault, path: meta.path, content });
      await refresh();
      organizeOpen = false;
      organizeResult = '';
      organizeInstruction = '';
    } catch (error) {
      organizeError = error instanceof Error ? error.message : String(error);
    }
  }
</script>

<section class="flex h-full min-h-0 gap-0">
  {#if !vault}
    <div class="flex h-full w-full flex-col items-center justify-center gap-3">
      <p class="text-sm text-muted">还没有笔记库。初始化会在「文档\千寻笔记」创建目录。</p>
      <button
        class="rounded-md bg-accent px-4 py-2 text-sm font-medium text-white hover:bg-accent/90"
        onclick={() => void initVault()}
      >
        初始化笔记库
      </button>
      {#if errorText}<p class="text-sm text-danger">{errorText}</p>{/if}
    </div>
  {:else}
    <!-- 列表 -->
    <aside class="flex w-64 shrink-0 flex-col border-r border-line">
      <div class="flex items-center gap-2 border-b border-line p-2">
        <input
          class="min-w-0 flex-1 rounded-md border border-line bg-surface px-2 py-1 text-sm"
          type="text"
          placeholder="标题/标签过滤"
          bind:value={filter}
        />
        <button
          class="shrink-0 rounded-md bg-accent px-2 py-1 text-sm text-white hover:bg-accent/90"
          title="新建笔记"
          onclick={() => void create()}
        >
          +
        </button>
      </div>
      <ul class="min-h-0 flex-1 overflow-y-auto">
        {#each filtered as note (note.path)}
          <li>
            <button
              class="w-full border-b border-line/50 px-3 py-2 text-left transition-colors hover:bg-accent-soft/60 {activePath ===
              note.path
                ? 'bg-accent-soft'
                : ''}"
              onclick={() => void open(note)}
            >
              <p class="truncate text-sm">{note.title}</p>
              <p class="mt-0.5 flex items-center gap-2 text-xs text-muted">
                <span>{formatDate(note.updated)}</span>
                {#each note.tags.slice(0, 3) as tag (tag)}
                  <span class="rounded bg-accent-soft px-1">{tag}</span>
                {/each}
              </p>
            </button>
          </li>
        {/each}
      </ul>
    </aside>

    <!-- 编辑 / 预览 -->
    <div class="flex min-w-0 flex-1 flex-col">
      <div class="flex items-center gap-2 border-b border-line bg-surface px-3 py-1.5 text-xs">
        <span class="truncate text-muted">{activeNote?.meta.path ?? '未选择笔记'}</span>
        {#if dirty}<span class="text-accent">未保存</span>{/if}
        <span class="ml-auto flex items-center gap-2">
          <button
            class="rounded px-2 py-1 hover:bg-accent-soft disabled:opacity-40"
            title={dshOrigin ? 'AI 整理（经 qx-bridge）' : 'DSH 未运行：先启动 DSH 并部署桥'}
            disabled={!dshOrigin || organizing}
            onclick={() => {
              organizeOpen = !organizeOpen;
              organizeError = '';
            }}
          >
            {organizing ? '整理中…' : 'AI 整理'}
          </button>
          <button
            class="rounded px-2 py-1 hover:bg-accent-soft disabled:opacity-40"
            disabled={!activeNote || saving}
            onclick={() => void save()}
          >
            保存
          </button>
          <button
            class="rounded px-2 py-1 hover:bg-accent-soft disabled:opacity-40"
            disabled={!activeNote}
            onclick={() => (previewing = !previewing)}
          >
            {previewing ? '编辑' : '预览'}
          </button>
          <button
            class="rounded px-2 py-1 text-danger hover:bg-danger/10 disabled:opacity-40"
            disabled={!activeNote}
            onclick={() => void remove()}
          >
            删除
          </button>
        </span>
      </div>
      {#if organizeOpen}
        <div class="flex shrink-0 flex-col gap-2 border-b border-line bg-surface px-3 py-2">
          <div class="flex items-center gap-2">
            <input
              class="min-w-0 flex-1 rounded-md border border-line bg-bg px-2 py-1 text-xs"
              type="text"
              placeholder="整理指令，如：把所有笔记里的 Rust 命令合并成一篇速查表"
              bind:value={organizeInstruction}
            />
            <button
              class="rounded bg-accent px-2.5 py-1 text-xs text-white hover:bg-accent/90 disabled:opacity-40"
              disabled={organizing || !dshOrigin}
              onclick={() => void runOrganize()}
            >
              {organizing ? '生成中…' : '生成'}
            </button>
            {#if organizeResult}
              <button
                class="rounded px-2.5 py-1 text-xs hover:bg-accent-soft"
                onclick={() => void saveOrganizeAsNote()}
              >
                存为笔记
              </button>
            {/if}
            <button
              class="rounded px-2 py-1 text-xs text-muted hover:bg-accent-soft"
              onclick={() => (organizeOpen = false)}
            >
              收起
            </button>
          </div>
          {#if !dshOrigin}
            <p class="text-xs text-muted">DSH 未运行或桥未部署：先启动 DSH（并在设置页部署桥）。</p>
          {/if}
          {#if organizeError}<p class="text-xs text-danger">{organizeError}</p>{/if}
          {#if organizeResult}
            <textarea
              class="h-40 resize-y rounded-md border border-line bg-bg px-2 py-1 font-mono text-xs leading-relaxed"
              readonly>{organizeResult}</textarea
            >
          {/if}
        </div>
      {/if}
      {#if activeNote}
        {#if previewing}
          <div class="prose-notes min-h-0 flex-1 overflow-y-auto p-6">
            <!-- 个人笔记库，内容全部自产（本人撰写/AI 整理回写），无第三方注入面：豁免 XSS lint -->
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            {@html previewHtml}
          </div>
        {:else}
          <div class="min-h-0 flex-1 overflow-hidden">
            <div class="h-full overflow-auto" bind:this={host}></div>
          </div>
        {/if}
      {:else}
        <div class="flex flex-1 items-center justify-center text-sm text-muted">
          左侧选择或新建一篇笔记
        </div>
      {/if}
      {#if errorText}<p class="border-t border-line px-3 py-1.5 text-xs text-danger">
          {errorText}
        </p>{/if}
    </div>
  {/if}
</section>

<style>
  /* 笔记预览的朴素排版（不引 tailwind typography，个人工具够用）。 */
  .prose-notes :global(h1) {
    font-size: 1.5rem;
    font-weight: 600;
    margin: 1rem 0 0.5rem;
  }
  .prose-notes :global(h2) {
    font-size: 1.25rem;
    font-weight: 600;
    margin: 1rem 0 0.5rem;
  }
  .prose-notes :global(h3) {
    font-size: 1.1rem;
    font-weight: 600;
    margin: 0.8rem 0 0.4rem;
  }
  .prose-notes :global(p) {
    margin: 0.5rem 0;
    line-height: 1.7;
  }
  .prose-notes :global(ul),
  .prose-notes :global(ol) {
    padding-left: 1.5rem;
    margin: 0.5rem 0;
  }
  .prose-notes :global(ul) {
    list-style: disc;
  }
  .prose-notes :global(ol) {
    list-style: decimal;
  }
  .prose-notes :global(code) {
    background: rgba(127, 127, 127, 0.15);
    border-radius: 4px;
    padding: 0.1rem 0.3rem;
    font-family: 'Cascadia Mono', Consolas, monospace;
    font-size: 0.875em;
  }
  .prose-notes :global(pre) {
    background: rgba(127, 127, 127, 0.12);
    border-radius: 8px;
    padding: 0.75rem 1rem;
    overflow-x: auto;
    margin: 0.6rem 0;
  }
  .prose-notes :global(pre code) {
    background: none;
    padding: 0;
  }
  .prose-notes :global(blockquote) {
    border-left: 3px solid rgba(127, 127, 127, 0.4);
    padding-left: 0.8rem;
    color: inherit;
    opacity: 0.8;
    margin: 0.5rem 0;
  }
  .prose-notes :global(a) {
    color: #3b82f6;
    text-decoration: underline;
  }
  .prose-notes :global(table) {
    border-collapse: collapse;
    margin: 0.6rem 0;
  }
  .prose-notes :global(th),
  .prose-notes :global(td) {
    border: 1px solid rgba(127, 127, 127, 0.35);
    padding: 0.3rem 0.6rem;
  }
  .prose-notes :global(img) {
    max-width: 100%;
  }
</style>
