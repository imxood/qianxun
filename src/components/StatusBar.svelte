<script lang="ts">
  import { onMount } from 'svelte';
  import { call } from '../lib/ipc';
  import type { AppMetaResult, ThemePreference } from '../lib/ipc/contract';
  import { settings } from '../stores/settings.svelte';
  import { harness } from '../stores/harness.svelte';
  import { theme } from '../stores/theme.svelte';

  let meta: AppMetaResult | null = $state(null);
  let metaError: string | null = $state(null);

  const themeCycle: ThemePreference[] = ['system', 'light', 'dark'];
  const themeLabel: Record<ThemePreference, string> = {
    system: '跟随系统',
    light: '浅色',
    dark: '深色',
  };
  /** 主题切换按钮的小图标（system=显示器 / light=太阳 / dark=月亮）。 */
  const themeIcon: Record<ThemePreference, string> = {
    system: 'M4 5h16v11H4zM9 20h6M12 16v4',
    light:
      'M12 7a5 5 0 100 10 5 5 0 000-10zM12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4',
    dark: 'M20 13.5A8 8 0 1110.5 4 6.5 6.5 0 0020 13.5z',
  };

  const statusLabel: Record<string, string> = {
    stopped: 'DSH 未运行',
    starting: 'DSH 启动中…',
    ready: 'DSH 运行中',
    restarting: 'DSH 重启中…',
    failed: 'DSH 启动失败',
  };
  const statusTone: Record<string, string> = {
    stopped: 'text-muted',
    starting: 'text-accent',
    ready: 'text-ok',
    restarting: 'text-accent',
    failed: 'text-danger',
  };

  onMount(() => {
    void (async () => {
      try {
        meta = await call<AppMetaResult>('app_meta');
      } catch (error) {
        metaError = error instanceof Error ? error.message : String(error);
      }
    })();
    void harness.wire();
  });

  function cycleTheme(): void {
    const index = themeCycle.indexOf(theme.preference);
    const next = themeCycle[(index + 1) % themeCycle.length] ?? 'system';
    // 本地立即生效，持久化异步进行；失败时设置页会显示加载/保存错误。
    theme.set(next);
    void settings.update({ theme: next });
  }
</script>

<footer
  class="flex h-7 shrink-0 items-center justify-between border-t border-line bg-surface pr-2 pl-3 text-xs text-muted"
>
  <span class="tabular-nums"
    >{meta ? `千寻 v${meta.version}` : metaError ? '版本获取失败' : '千寻'}</span
  >
  <div class="flex items-center gap-4">
    <span class="flex items-center gap-1.5 {statusTone[harness.status.phase] ?? 'text-muted'}">
      <span
        class="size-1.5 rounded-full bg-current {harness.status.phase === 'ready'
          ? 'shadow-[0_0_8px_currentColor]'
          : ''} {harness.status.phase === 'ready' ||
        harness.status.phase === 'starting' ||
        harness.status.phase === 'restarting'
          ? 'animate-pulse'
          : ''}"
        aria-hidden="true"
      ></span>
      {statusLabel[harness.status.phase] ?? 'DSH'}
      {#if harness.status.phase === 'ready'}<span class="opacity-70">· {harness.status.origin}</span
        >{/if}
    </span>
    <button
      class="flex items-center gap-1 rounded px-1.5 py-0.5 transition-colors outline-none hover:bg-accent-soft hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/50"
      onclick={cycleTheme}
      title="切换主题"
    >
      <svg
        viewBox="0 0 24 24"
        class="size-3"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d={themeIcon[theme.preference]} />
      </svg>
      {themeLabel[theme.preference]}
    </button>
  </div>
</footer>
