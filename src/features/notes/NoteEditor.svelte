<script lang="ts">
  /**
   * Markdown 编辑器（CodeMirror 6 封装）：命令式库只准存在于这一层（规范 §5）。
   * 挂载生命周期用 $effect 跟随 host 容器——容器从无到有（打开笔记）、
   * 预览切换后重建，编辑器都随之正确重建；这替代了原实现「onMount 时
   * parent: host! 传 null → 游离视图」的结构性缺陷。
   */
  import { EditorState, Compartment } from '@codemirror/state';
  import { EditorView, basicSetup } from 'codemirror';
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
  import { languages } from '@codemirror/language-data';
  import { oneDark } from '@codemirror/theme-one-dark';

  let {
    doc = '',
    dark = false,
    onDocChange,
  }: {
    doc?: string;
    dark?: boolean;
    onDocChange: (next: string) => void;
  } = $props();

  let host: HTMLDivElement | null = $state(null);
  // 非 $state：纯命令式引用，不需参与响应。
  let view: EditorView | null = null;
  const themeSlot = new Compartment();

  // 挂载/销毁跟随容器。
  $effect(() => {
    if (!host) return;
    const created = new EditorView({
      state: EditorState.create({
        doc,
        extensions: [
          basicSetup,
          EditorView.lineWrapping,
          markdown({ codeLanguages: languages, base: markdownLanguage }),
          themeSlot.of(dark ? oneDark : []),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onDocChange(update.state.doc.toString());
          }),
        ],
      }),
      parent: host,
    });
    view = created;
    return () => {
      view = null;
      created.destroy();
    };
  });

  // 外部 doc 变化（打开另一篇）整体替换；用户输入回流的同内容不触发——
  // 比较「传入 doc vs 编辑器现内容」而非引用，光标不因回流被重置。
  $effect(() => {
    const next = doc;
    if (view && next !== view.state.doc.toString()) {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: next } });
    }
  });

  // 深/浅主题热切换（Compartment 原地重配，不重建视图）。
  $effect(() => {
    view?.dispatch({ effects: themeSlot.reconfigure(dark ? oneDark : []) });
  });

  export function focus(): void {
    view?.focus();
  }
</script>

<div class="h-full overflow-auto" bind:this={host}></div>
