<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import TitleBar from './components/TitleBar.svelte';
  import SideNav from './components/SideNav.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import ContextMenuLayer from './components/ContextMenuLayer.svelte';
  import OverviewPage from './features/overview/OverviewPage.svelte';
  import EnvPage from './features/env/EnvPage.svelte';
  import ConsolePage from './features/console/ConsolePage.svelte';
  import DshPage from './features/dsh/DshPage.svelte';
  import FilesPage from './features/search/FilesPage.svelte';
  import GrepPage from './features/search/GrepPage.svelte';
  import TerminalPage from './features/terminal/TerminalPage.svelte';
  import NotesPage from './features/notes/NotesPage.svelte';
  import SettingsPage from './features/settings/SettingsPage.svelte';
  import type { PageId } from './stores/nav.svelte';
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
    return () => {
      harness.dispose();
      search.dispose();
    };
  });

  // 设置到达后同步主题偏好（启动时与每次保存后各一次）。
  $effect(() => {
    if (settings.current) theme.set(settings.current.theme);
  });

  // 生效主题 → DOM 类切换。theme.set 与系统偏好变化都会驱动这里。
  $effect(() => {
    document.documentElement.classList.toggle('dark', theme.resolved === 'dark');
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

<div class="flex h-full flex-col overflow-hidden bg-bg">
  <TitleBar />
  <div class="flex min-h-0 flex-1">
    <SideNav />
    <!-- 所有页面绝对定位叠放在同一容器里，同一时刻只有一页可见。 -->
    <div class="relative min-w-0 flex-1">
      {#if nav.visited.dsh}
        <div class="absolute inset-0 overflow-hidden {show('dsh')}">
          <DshPage />
        </div>
      {/if}
      {#if nav.visited.terminal}
        <div class="absolute inset-0 overflow-hidden {show('terminal')}">
          <TerminalPage />
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
        <div class="absolute inset-0 overflow-y-auto p-6 {show('env')}">
          <EnvPage />
        </div>
      {/if}
      {#if nav.visited.console}
        <div class="absolute inset-0 overflow-y-auto p-6 {show('console')}">
          <ConsolePage />
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
