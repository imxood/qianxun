<script lang="ts">
  /**
   * 单个终端面板：一个 xterm 实例 ↔ 一个 PTY 会话（id 由父层 spawn 分配）。
   * 标签切换由父层 CSS 隐藏保活（组件不销毁，回滚与进程状态保留）。
   * 输出可靠性的两道保险：挂载后 terminal_replay 回放（弥合 spawn→监听
   * 注册窗口期丢失的横幅/提示符）；keep-alive 重见时主动 fit（visibility
   * 切换不触发 ResizeObserver）。
   *
   * 右键菜单：复制（有选中时）/ 粘贴 / 清空。OSC 7（Rust 侧 prompt 钩子
   * 注入）上报 cwd 供 PIN 恢复。
   */
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import '@xterm/xterm/css/xterm.css';
  import { listen } from '@tauri-apps/api/event';
  import { call } from '../../lib/ipc';
  import { contextMenu } from '../../lib/menu.svelte';
  import { osc633CwdToWindows, osc99ToWindows, oscPathToWindows } from '../../lib/utils/osc';
  import type {
    TerminalExitEvent,
    TerminalOutputEvent,
    TerminalSettings,
  } from '../../lib/ipc/contract';

  export interface PaneApi {
    clear(): void;
    paste(): void;
    focus(): void;
    hasSelection(): boolean;
    copySelection(): boolean;
  }

  let {
    id,
    active,
    prefs,
    shell = null,
    initialHistory = '',
    onExit,
    onTitle,
    onCwd,
    onBind,
  }: {
    id: number;
    active: boolean;
    prefs: TerminalSettings;
    /** 会话的实际 shell（选清屏命令用；未知按 POSIX clear 处理）。 */
    shell: string | null;
    /** 恢复的固定终端：启动时写进 xterm 的历史内容。 */
    initialHistory?: string;
    onExit: (id: number) => void;
    onTitle: (id: number, title: string) => void;
    onCwd: (id: number, cwd: string) => void;
    onBind: (id: number, api: PaneApi) => void;
  } = $props();

  let host: HTMLDivElement | null = $state(null);
  let alive = $state(true);

  // 命令式引用：onMount 内定义，$effect/模板回调按需调用。
  let syncSize: (() => void) | null = null;
  let pasteFromClipboard: (() => void) | null = null;
  let clearPane: (() => void) | null = null;
  let selectionText: (() => string) | null = null;
  let clearSelection: (() => void) | null = null;
  /** xterm 实例（onMount 创建；热应用设置的顶层 effect 引用）。 */
  let terminalRef: Terminal | null = null;

  // 终端设置热应用：标签条设置按钮改动后，已开标签即时生效（含
  // scrollback——xterm 原生支持收缩/扩张），字号变化后补一次 fit。
  // terminalRef 就绪前跳过；顶层 effect（onMount 里不能建）。
  $effect(() => {
    const terminal = terminalRef;
    if (!terminal) return;
    terminal.options.fontSize = prefs.fontSize;
    terminal.options.cursorStyle = prefs.cursorStyle;
    terminal.options.cursorBlink = prefs.cursorBlink;
    terminal.options.scrollback = prefs.scrollback;
    requestAnimationFrame(() => syncSize?.());
  });

  // keep-alive 重见：visibility 切换不触发 ResizeObserver，主动补一次 fit。
  $effect(() => {
    if (!active) return;
    requestAnimationFrame(() => syncSize?.());
  });

  /** OSC 7 的 file:// URL → Windows 路径（纯函数在 lib/utils/osc，带单测）。 */

  /**
   * 右键菜单：有选区 → 复制；粘贴（剪贴板为空/不可读时灰显）；清空。
   * 剪贴板经 Rust 侧插件读写（clipboard_read_text）：不经 WebView2 的
   * navigator.clipboard，不会弹系统权限框。菜单点击后焦点交回终端
   * （菜单按钮吃掉焦点，关闭后不回）。
   */
  async function menu(event: MouseEvent): Promise<void> {
    const selection = selectionText?.() ?? '';
    let clipboard: string | null;
    try {
      clipboard = await call<string>('clipboard_read_text');
    } catch {
      clipboard = null; // 读取失败：粘贴必然失败，菜单里按禁用呈现。
    }
    const pasteDisabled = clipboard === null || clipboard.length === 0;
    const refocus = (): void => terminalRef?.focus();

    const items: Array<{ label: string; onclick?: () => void; disabled?: boolean }> = [];
    if (selection) {
      items.push({
        label: '复制',
        onclick: () => {
          void copyText(selection);
          refocus();
        },
      });
    }
    items.push({
      label: '粘贴',
      disabled: pasteDisabled,
      onclick: () => {
        pasteFromClipboard?.();
        refocus();
      },
    });
    items.push({
      label: '清空',
      onclick: () => {
        clearPane?.();
        refocus();
      },
    });
    contextMenu.show(event, items);
  }

  /** 写剪贴板（走主进程插件，避免 WebView2 权限弹窗）。 */
  async function copyText(text: string): Promise<void> {
    try {
      await call('clipboard_write_text', { text });
    } catch {
      // 写失败静默：复制不是关键路径。
    }
  }

  /**
   * 点击选中内容即复制（Windows Terminal 习惯）：左键在已有选区上
   * 按下 → 复制并清除选区，且拦截掉 xterm 的默认 mousedown（否则它
   * 会立刻开始新的选区）。带修饰键的点击留给扩展选区语义。
   */
  function onHostMouseDownCapture(event: MouseEvent): void {
    if (event.button !== 0 || event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) {
      return;
    }
    const text = selectionText?.();
    if (!text) return;
    void copyText(text);
    clearSelection?.();
    event.stopPropagation();
    event.preventDefault();
  }

  onMount(() => {
    const terminal = new Terminal({
      fontSize: prefs.fontSize,
      scrollback: prefs.scrollback,
      fontFamily: '"Cascadia Mono", Consolas, "Courier New", monospace',
      cursorStyle: prefs.cursorStyle,
      cursorBlink: prefs.cursorBlink,
      theme: {
        background: '#1e1e1e',
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(host!);
    // webgl 加速；上下文获取失败（虚拟机/远程会话）自动降级 DOM 渲染。
    try {
      terminal.loadAddon(new WebglAddon());
    } catch {
      // DOM 渲染兜底，无需处理。
    }
    fit.fit();
    terminalRef = terminal;

    // 恢复的固定终端：先写历史，再等实时回放（新会话横幅接在后面）。
    if (initialHistory) terminal.write(initialHistory);

    // 剪贴板：Ctrl+Shift+C/V + 右键菜单（经主进程插件，无权限弹窗）。
    const doPaste = (): void => {
      call<string>('clipboard_read_text')
        .then((text) => {
          if (text) {
            void call('terminal_write', { id, data: text });
            terminal.focus(); // 粘贴后焦点留在终端（菜单点击会吃掉焦点）。
          }
        })
        .catch(() => {}); // 读取失败：静默，不影响键盘输入。
    };
    pasteFromClipboard = doPaste;
    selectionText = () => terminal.getSelection();
    clearSelection = () => terminal.clearSelection();
    // 点击选中内容即复制：capture 阶段在宿主上拦下，先于 xterm 的
    // mousedown 处理（否则选区立刻被新选择取代）。
    host!.addEventListener('mousedown', onHostMouseDownCapture, true);
    // 清屏交给 shell 自己执行（clear/cls）：它会重绘提示符，当前输入行
    // 原样保留；直接写转义码会连提示符一起抹掉且 shell 不知情。
    const basename = shell?.replaceAll('\\', '/').split('/').pop()?.toLowerCase() ?? '';
    const clearCommand = basename === 'cmd.exe' ? 'cls' : 'clear';
    clearPane = (): void => {
      void call('terminal_write', { id, data: `${clearCommand}\r` });
      // Rust 侧回放缓冲一起清，重放/恢复不再带旧内容（当前行的提示符
      // 由 shell 重绘产生，自然进入新回放）。
      void call('terminal_clear', { id }).catch(() => {});
      terminal.focus();
    };
    onBind(id, {
      clear: () => clearPane?.(),
      paste: () => doPaste(),
      focus: () => terminal.focus(),
      hasSelection: () => terminal.hasSelection(),
      copySelection: () => {
        const selection = terminal.getSelection();
        if (!selection) return false;
        void copyText(selection);
        return true;
      },
    });
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown') return true;
      if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 'c') {
        const selection = terminal.getSelection();
        if (selection) {
          void copyText(selection);
          return false; // 已由我们复制，无需浏览器接手。
        }
        // 无选中：交给浏览器（这里不是 devtools 快捷键的拦截点）。
        return true;
      }
      if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 'v') {
        // preventDefault 挡掉 WebView2 的「原样粘贴」编辑命令——否则
        // 浏览器向隐藏 textarea 插入一次 + 我们向 PTY 写一次 = 双重粘贴。
        event.preventDefault();
        event.stopPropagation();
        doPaste();
        return false;
      }
      return true;
    });

    terminal.onData((data) => {
      void call('terminal_write', { id, data });
    });

    // cwd 上报的三条通道（shell 各有所好，都接）：
    // - OSC 7：pwsh prompt 钩子（file:// URL）；
    // - OSC 9;9：nushell/ConEmu 工作目录（nushell 默认 osc7=false，这条
    //   是它的主通道）；
    // - OSC 633：VS Code shell 集成（nushell 默认开，属性里带 Cwd）。
    // 返回 false 让其他处理器继续（xterm 默认无这些处理器）。
    const reportCwd = (cwd: string): void => onCwd(id, cwd);
    terminal.parser.registerOscHandler(7, (data) => {
      const cwd = oscPathToWindows(data);
      if (cwd) reportCwd(cwd);
      return false;
    });
    terminal.parser.registerOscHandler(9, (data) => {
      const cwd = osc99ToWindows(data);
      if (cwd) reportCwd(cwd);
      return false;
    });
    terminal.parser.registerOscHandler(633, (data) => {
      const cwd = osc633CwdToWindows(data);
      if (cwd) reportCwd(cwd);
      return false;
    });

    const disposers: Array<() => void> = [];
    void (async () => {
      const unlistenOutput = await listen<TerminalOutputEvent>('terminal://output', (event) => {
        if (event.payload.id === id) terminal.write(event.payload.data);
      });
      const unlistenExit = await listen<TerminalExitEvent>('terminal://exit', (event) => {
        if (event.payload.id !== id) return;
        terminal.write(
          event.payload.exitCode === null || event.payload.exitCode === 0
            ? '\r\n\x1b[90m[进程已退出]\x1b[0m\r\n'
            : `\r\n\x1b[90m[进程已退出，代码 ${event.payload.exitCode}]\x1b[0m\r\n`,
        );
        unlistenOutput();
        unlistenExit();
        alive = false;
        onExit(id);
      });
      disposers.push(unlistenOutput, unlistenExit);
      // 回放：监听已就绪，补齐 spawn 以来的输出（含横幅与首个提示符）。
      try {
        const replayed = await call<string>('terminal_replay', { id });
        if (replayed && alive) terminal.write(replayed);
      } catch {
        // 会话已退出：exit 事件已收尾，回放落空无害。
      }
    })();

    // 标题随 shell 的 OSC 标题序列更新（提示符路径等）。
    terminal.onTitleChange((title) => {
      if (title) onTitle(id, title);
    });

    // 尺寸联动：fit 后把逻辑行列回传 PTY（初次 fit 也要同步）。
    const doSyncSize = (): void => {
      if (!alive) return;
      try {
        fit.fit();
      } catch {
        return; // 容器隐藏时 fit 会抛错：重新显示时由 $effect 再触发。
      }
      void call('terminal_resize', { id, cols: terminal.cols, rows: terminal.rows });
    };
    syncSize = doSyncSize;
    doSyncSize();
    const observer = new ResizeObserver(doSyncSize);
    observer.observe(host!);

    return () => {
      alive = false;
      observer.disconnect();
      for (const dispose of disposers) dispose();
      terminal.dispose();
    };
  });
</script>

<div
  class="h-full w-full bg-[#1e1e1e] {alive ? '' : 'opacity-80'}"
  bind:this={host}
  oncontextmenu={(event) => {
    event.preventDefault();
    menu(event);
  }}
></div>
