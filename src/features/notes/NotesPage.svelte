<script lang="ts">
  /**
   * 笔记页（M5）：列表 | CodeMirror 编辑 | Markdown 预览。
   * 纯文件库（ADR-006）；保存走原子写；删除入 .trash 可救。
   * v0.2：编辑器容器常驻（预览切换不销毁）、frontmatter 由 Rust 拼装
   * （notes_save 收结构化 title/tags）、自动保存（1.5s 防抖+失焦+Ctrl+S）、
   * 对话框组件化（去 window.prompt/confirm）。
   */
  import { onMount, tick } from 'svelte';
  import { marked } from 'marked';
  import { call } from '../../lib/ipc';
  import type { NoteContent, NoteMeta } from '../../lib/ipc/contract';
  import { settings } from '../../stores/settings.svelte';
  import { harness } from '../../stores/harness.svelte';
  import { theme } from '../../stores/theme.svelte';
  import ConfirmDialog from '../../components/ConfirmDialog.svelte';
  import PromptDialog from '../../components/PromptDialog.svelte';
  import NoteEditor from './NoteEditor.svelte';

  let notes = $state<NoteMeta[]>([]);
  let filter = $state('');
  let activePath = $state<string | null>(null);
  let activeNote = $state<NoteContent | null>(null);
  let dirty = $state(false);
  let saving = $state(false);
  let errorText = $state('');
  let previewing = $state(false);
  let editor: NoteEditor | null = $state(null);
  // 结构化元数据编辑（frontmatter 的 UI 形态，用户不手写 YAML）。
  let editTitle = $state('');
  let editTags = $state('');

  // ---- 对话框状态 ----
  let createOpen = $state(false);
  let removeOpen = $state(false);
  let discardOpen = $state(false);
  let saveAsTitleOpen = $state(false);

  // ---- AI 整理（M6：经 qx-bridge 的 /qx/notes/organize） ----
  let organizing = $state(false);
  let organizeOpen = $state(false);
  let organizeInstruction = $state('');
  let organizeResult = $state('');
  let organizeError = $state('');
  const dshOrigin = $derived(harness.status.phase === 'ready' ? harness.status.origin : '');

  const vault = $derived(settings.current?.notes.vaultDir ?? '');
  const dark = $derived(theme.resolved === 'dark');
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

  // 库目录就绪（或首次初始化）后拉清单。
  $effect(() => {
    if (vault) void refresh();
  });

  // 外部变更感知（轻量）：窗口重获焦点时静默刷新清单，不做 watcher。
  $effect(() => {
    if (!vault) return;
    const onFocus = (): void => {
      void refresh();
    };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  });

  // 自动保存：Ctrl+S 手动兜底。
  function onKeydown(event: KeyboardEvent): void {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') {
      event.preventDefault();
      if (dirty && !saving) void save();
    }
  }

  let saveTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => {
    // 自动保存防抖句柄的最终清理。
    return () => {
      if (saveTimer !== null) clearTimeout(saveTimer);
    };
  });

  function markDirty(): void {
    dirty = true;
    if (saveTimer !== null) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      if (dirty && !saving) void save();
    }, 1500);
  }

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

  function parseTags(raw: string): string[] {
    return raw
      .split(/[,，]/)
      .map((tag) => tag.trim())
      .filter(Boolean);
  }

  async function open(note: NoteMeta, force = false): Promise<void> {
    if (dirty && !force) {
      pendingOpen = note;
      discardOpen = true;
      return;
    }
    errorText = '';
    try {
      const content = await call<NoteContent>('notes_read', { vault, path: note.path });
      activeNote = content;
      activePath = note.path;
      editTitle = content.meta.title;
      editTags = content.meta.tags.join(', ');
      dirty = false;
      previewing = false;
      await tick();
      editor?.focus();
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  let pendingOpen: NoteMeta | null = null;

  function confirmDiscard(): void {
    const note = pendingOpen;
    pendingOpen = null;
    dirty = false;
    if (note) void open(note, true);
  }

  async function save(): Promise<void> {
    if (!activeNote || saving) return;
    saving = true;
    errorText = '';
    try {
      const meta = await call<NoteMeta>('notes_save', {
        vault,
        path: activeNote.meta.path,
        title: editTitle.trim() || activeNote.meta.title,
        tags: parseTags(editTags),
        body: activeNote.body,
      });
      activeNote = { ...activeNote, meta };
      editTitle = meta.title;
      editTags = meta.tags.join(', ');
      dirty = false;
      await refresh();
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    } finally {
      saving = false;
    }
  }

  async function create(title: string): Promise<void> {
    errorText = '';
    try {
      const meta = await call<NoteMeta>('notes_create', { vault, title });
      await refresh();
      await open(meta, true);
    } catch (error) {
      errorText = error instanceof Error ? error.message : String(error);
    }
  }

  async function remove(): Promise<void> {
    if (!activeNote) return;
    errorText = '';
    try {
      await call('notes_delete', { vault, path: activeNote.meta.path });
      activeNote = null;
      activePath = null;
      dirty = false;
      previewing = false;
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

  function formatRelative(stamp: number): string {
    const diff = Date.now() - stamp;
    const minute = 60_000;
    const hour = 3_600_000;
    const day = 86_400_000;
    if (diff < minute) return '刚刚';
    if (diff < hour) return `${Math.floor(diff / minute)} 分钟前`;
    if (diff < day) return `${Math.floor(diff / hour)} 小时前`;
    if (diff < 7 * day) return `${Math.floor(diff / day)} 天前`;
    return formatDate(stamp);
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

  async function saveOrganizeAsNote(title: string): Promise<void> {
    if (!organizeResult.trim()) return;
    try {
      const meta = await call<NoteMeta>('notes_create', { vault, title });
      // frontmatter 由 Rust 拼装：直接给结构化字段。
      await call('notes_save', {
        vault,
        path: meta.path,
        title,
        tags: ['ai-整理'],
        body: organizeResult,
      });
      await refresh();
      organizeOpen = false;
      organizeResult = '';
      organizeInstruction = '';
    } catch (error) {
      organizeError = error instanceof Error ? error.message : String(error);
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

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
          onclick={() => (createOpen = true)}
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
              <p class="mt-0.5 truncate text-xs text-muted">{note.excerpt}</p>
              <p class="mt-0.5 flex items-center gap-2 text-xs text-muted">
                <span>{formatRelative(note.updated)}</span>
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
            onclick={() => (removeOpen = true)}
          >
            删除
          </button>
        </span>
      </div>
      {#if activeNote}
        <!-- 结构化元数据：frontmatter 的 UI 形态（修改即标脏，随保存写回）。 -->
        <div class="flex items-center gap-2 border-b border-line bg-surface px-3 py-1.5">
          <input
            class="min-w-0 flex-1 rounded-md border border-transparent bg-transparent px-1.5 py-1 text-sm font-medium outline-none focus:border-line"
            placeholder="标题"
            bind:value={editTitle}
            oninput={markDirty}
          />
          <input
            class="w-64 shrink-0 rounded-md border border-transparent bg-transparent px-1.5 py-1 text-xs outline-none focus:border-line"
            placeholder="标签（逗号分隔）"
            bind:value={editTags}
            oninput={markDirty}
          />
        </div>
      {/if}
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
                onclick={() => (saveAsTitleOpen = true)}
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
        <!-- 编辑器容器常驻 DOM（hidden 切换）：预览来回切不销毁 CodeMirror。 -->
        <div class="min-h-0 flex-1 overflow-hidden {previewing ? 'hidden' : ''}">
          <NoteEditor
            bind:this={editor}
            doc={activeNote.body}
            {dark}
            onDocChange={(next) => {
              if (activeNote && next !== activeNote.body) {
                activeNote = { ...activeNote, body: next };
                markDirty();
              }
            }}
          />
        </div>
        {#if previewing}
          <div class="prose-notes min-h-0 flex-1 overflow-y-auto p-6">
            <!-- 个人笔记库，内容全部自产（本人撰写/AI 整理回写），无第三方注入面：豁免 XSS lint -->
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            {@html previewHtml}
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

<PromptDialog
  open={createOpen}
  title="新建笔记"
  label="标题"
  placeholder="如：Rust 生命周期速记"
  confirmLabel="创建"
  onconfirm={(title) => {
    createOpen = false;
    void create(title);
  }}
  oncancel={() => (createOpen = false)}
/>

<PromptDialog
  open={saveAsTitleOpen}
  title="存为笔记"
  label="标题"
  initialValue="AI 整理稿"
  confirmLabel="保存"
  onconfirm={(title) => {
    saveAsTitleOpen = false;
    void saveOrganizeAsNote(title);
  }}
  oncancel={() => (saveAsTitleOpen = false)}
/>

<ConfirmDialog
  open={removeOpen}
  title="删除笔记"
  message={activeNote
    ? `删除「${editTitle || activeNote.meta.title}」？（移入 .trash 可找回）`
    : ''}
  confirmLabel="删除"
  danger
  onconfirm={() => {
    removeOpen = false;
    void remove();
  }}
  oncancel={() => (removeOpen = false)}
/>

<ConfirmDialog
  open={discardOpen}
  title="未保存的修改"
  message="当前笔记有未保存的修改，切换将丢弃。继续？"
  confirmLabel="丢弃并切换"
  danger
  onconfirm={() => {
    discardOpen = false;
    confirmDiscard();
  }}
  oncancel={() => {
    discardOpen = false;
    pendingOpen = null;
  }}
/>

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
