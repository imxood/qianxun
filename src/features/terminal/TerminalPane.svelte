<script lang="ts">
  /**
   * 单个终端面板：一个 xterm 实例 ↔ 一个 PTY 会话（id 由父层 spawn 分配）。
   * 标签切换由父层 CSS 隐藏保活（组件不销毁，回滚与进程状态保留）。
   * 输出可靠性的两道保险：挂载后 terminal_replay 回放（弥合 spawn→监听
   * 注册窗口期丢失的横幅/提示符）；keep-alive 重见时主动 fit（visibility
   * 切换不触发 ResizeObserver）。
   */
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import { WebglAddon } from '@xterm/addon-webgl';
  import '@xterm/xterm/css/xterm.css';
  import { listen } from '@tauri-apps/api/event';
  import { call } from '../../lib/ipc';
  import type {
    TerminalExitEvent,
    TerminalOutputEvent,
    TerminalSettings,
  } from '../../lib/ipc/contract';

  let {
    id,
    active,
    prefs,
    onExit,
    onTitle,
  }: {
    id: number;
    active: boolean;
    prefs: TerminalSettings;
    onExit: (id: number) => void;
    onTitle: (id: number, title: string) => void;
  } = $props();

  let host: HTMLDivElement | null = $state(null);
  let alive = $state(true);

  // 命令式引用：onMount 内定义，$effect/模板回调按需调用。
  let syncSize: (() => void) | null = null;
  let pasteFromClipboard: (() => void) | null = null;

  // keep-alive 重见：visibility 切换不触发 ResizeObserver，主动补一次 fit。
  $effect(() => {
    if (!active) return;
    requestAnimationFrame(() => syncSize?.());
  });

  onMount(() => {
    const terminal = new Terminal({
      fontSize: prefs.fontSize,
      scrollback: prefs.scrollback,
      fontFamily: '"Cascadia Mono", Consolas, "Courier New", monospace',
      cursorBlink: true,
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

    // 剪贴板：Ctrl+Shift+C/V + 右键粘贴（WebView2 剪贴板权限策略下尽力而为）。
    const doPaste = (): void => {
      navigator.clipboard
        .readText()
        .then((text) => {
          if (text) void call('terminal_write', { id, data: text });
        })
        .catch(() => {}); // 权限/上下文不支持：静默，不影响键盘输入。
    };
    pasteFromClipboard = doPaste;
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown') return true;
      if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 'c') {
        const selection = terminal.getSelection();
        if (selection) {
          navigator.clipboard.writeText(selection).catch(() => {});
        }
        return false;
      }
      if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 'v') {
        doPaste();
        return false;
      }
      return true;
    });

    terminal.onData((data) => {
      void call('terminal_write', { id, data });
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
    pasteFromClipboard?.();
  }}
></div>
