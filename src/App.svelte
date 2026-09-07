<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { listen } from '@tauri-apps/api/event';
  import { call } from './lib/ipc';
  import TitleBar from './components/TitleBar.svelte';
  import SideNav from './components/SideNav.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import ContextMenuLayer from './components/ContextMenuLayer.svelte';
  import OverviewPage from './features/overview/OverviewPage.svelte';
  import EnvPage from './features/env/EnvPage.svelte';
  import DshPage from './features/dsh/DshPage.svelte';
  import FilesPage from './features/search/FilesPage.svelte';
  import GrepPage from './features/search/GrepPage.svelte';
  import TerminalPage from './features/terminal/TerminalPage.svelte';
  import RemotePage from './features/remote/RemotePage.svelte';
  import NotesPage from './features/notes/NotesPage.svelte';
  import SettingsPage from './features/settings/SettingsPage.svelte';
  import type { PageId } from './stores/nav.svelte';
  import type { StandaloneClosedEvent } from './lib/ipc/contract';
  import { nav } from './stores/nav.svelte';
  import { settings } from './stores/settings.svelte';
  import { harness } from './stores/harness.svelte';
  import { search } from './stores/search.svelte';
  import { theme } from './stores/theme.svelte';

  onMount(() => {
    // 主窗以隐藏创建（tauri.conf visible:false），前端就绪后由这里亮窗：
    // 几何恢复/最大化在隐藏期间已完成，用户看到的第一帧就是最终形态，
    // 不再出现「小白窗 → 切大窗 → 白屏」的两段式启动。startMinimized
    // 时保持隐藏（托盘唤起）；设置加载失败也要亮窗（fail-visible）。
    void (async () => {
      try {
        await settings.load();
      } finally {
        if (!settings.current?.window.startMinimized) {
          const main = getCurrentWindow();
          await main.show();
          await main.setFocus();
        }
      }
    })();
    // 事件线只建一次；页面轮转不重订阅。
    void harness.wire();
    // 独立窗口关闭 → 主窗恢复对应侧栏项与页面容器。
    const disposers: Array<() => void> = [];
    void listen<StandaloneClosedEvent>('window://closed', (event) => {
      nav.reattach(event.payload.view);
    }).then((unlisten) => disposers.push(unlisten));
    return () => {
      for (const dispose of disposers) dispose();
      harness.dispose();
      search.dispose();
    };
  });

  /**
   * 开发者工具的显式开关：浏览器加速键已在 Rust 侧关闭（Ctrl+Shift+C
   * 误触的根因），F12 在这里接管开与关（Rust 命令实现）。
   */
  function toggleDevtools(event: KeyboardEvent): void {
    if (event.key !== 'F12') return;
    event.preventDefault();
    event.stopPropagation();
    void call('app_toggle_devtools').catch(() => {});
  }

  // 设置到达后同步主题偏好（启动时与每次保存后各一次）。
  $effect(() => {
    if (settings.current) theme.set(settings.current.theme);
  });

  // 生效主题 → DOM 类切换 + localStorage 回写（下次启动的静态启动屏
  // 用它抢先上色，bundle 加载期间不再闪错色）。theme.set 与系统偏好
  // 变化都会驱动这里。
  $effect(() => {
    document.documentElement.classList.toggle('dark', theme.resolved === 'dark');
    try {
      localStorage.setItem('qx-theme', theme.resolved);
    } catch {
      /* 隐私模式等：回写失败无碍 */
    }
  });

  // 真 SPA keep-alive：首次进入才挂载，之后切页只显隐，状态全程保留。
  // DSH 额外后台预热——就绪即预载 iframe，第一次点击已是加载完成的页面。
  $effect(() => {
    nav.visit(nav.page);
    if (harness.status.phase === 'ready') nav.visit('dsh');
  });

  // 页面容器显隐：visibility（而非 display）保布局——滚动位置、xterm 尺寸、
  // iframe 文档全部原样保留；invisible 元素不接收指针事件、不进 Tab 焦点序。
  const show = (page: PageId): string => (nav.page === page ? '' : 'invisible');
</script>

<svelte:window onkeydown={toggleDevtools} />

<div class="flex h-full flex-col overflow-hidden bg-bg">
  <TitleBar />
  <div class="flex min-h-0 flex-1">
    <SideNav />
    <!-- 所有页面绝对定位叠放在同一容器里，同一时刻只有一页可见。
         已分离（detached）的页不渲染主窗副本——活视图在独立窗口里。 -->
    <div class="relative min-w-0 flex-1">
      {#if nav.visited.dsh && !nav.detached.dsh}
        <div class="absolute inset-0 overflow-hidden {show('dsh')}">
          <DshPage />
        </div>
      {/if}
      {#if nav.visited.terminal && !nav.detached.terminal}
        <div class="absolute inset-0 overflow-hidden {show('terminal')}">
          <TerminalPage />
        </div>
      {/if}
      {#if nav.visited.remote}
        <div class="absolute inset-0 overflow-y-auto p-6 {show('remote')}">
          <RemotePage />
        </div>
      {/if}
      {#if nav.visited.notes}
        <div class="absolute inset-0 overflow-hidden {show('notes')}">
          <NotesPage />
        </div>
      {/if}
      {#if nav.visited.overview}
        <div class="absolute inset-0 overflow-y-auto p-6 {show('overview')}">
          <OverviewPage />
        </div>
      {/if}
      {#if nav.visited.env}
        <div class="absolute inset-0 overflow-hidden {show('env')}">
          <EnvPage />
        </div>
      {/if}
      {#if nav.visited['search-files']}
        <div class="absolute inset-0 overflow-y-auto p-6 {show('search-files')}">
          <FilesPage />
        </div>
      {/if}
      {#if nav.visited['search-grep']}
        <div class="absolute inset-0 overflow-y-auto p-6 {show('search-grep')}">
          <GrepPage />
        </div>
      {/if}
      {#if nav.visited.settings}
        <div class="absolute inset-0 overflow-y-auto p-6 {show('settings')}">
          <SettingsPage />
        </div>
      {/if}
    </div>
  </div>
  <StatusBar />
  <ContextMenuLayer />
</div>
