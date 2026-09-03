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
  class="flex h-7 shrink-0 items-center justify-between border-t border-line bg-surface px-3 text-xs text-muted"
>
  <span>{meta ? `千寻 v${meta.version}` : metaError ? '版本获取失败' : '千寻'}</span>
  <div class="flex items-center gap-4">
    <span class={statusTone[harness.status.phase] ?? 'text-muted'}>
      {statusLabel[harness.status.phase] ?? 'DSH'}
      {#if harness.status.phase === 'ready'}· {harness.status.origin}{/if}
    </span>
    <button class="hover:text-fg" onclick={cycleTheme}>主题：{themeLabel[theme.preference]}</button>
  </div>
</footer>
