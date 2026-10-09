<script lang="ts">
  import { api, ApiError } from './lib/api';
  import { toast } from './lib/toast.svelte';
  import Login from './components/Login.svelte';
  import Overview from './components/Overview.svelte';
  import Devices from './components/Devices.svelte';
  import Audit from './components/Audit.svelte';
  import Toasts from './components/Toasts.svelte';

  type Page = 'overview' | 'devices' | 'audit';

  /** null = 还没问过 /me（首屏别闪登录页），true/false = 已定。 */
  let authed = $state<boolean | null>(null);
  let username = $state('');
  let page = $state<Page>('overview');

  const nav: { id: Page; label: string; icon: string }[] = [
    {
      id: 'overview',
      label: '概览',
      icon: '<rect x="3" y="3" width="7" height="9" rx="1.5"/><rect x="14" y="3" width="7" height="5" rx="1.5"/><rect x="14" y="12" width="7" height="9" rx="1.5"/><rect x="3" y="16" width="7" height="5" rx="1.5"/>',
    },
    {
      id: 'devices',
      label: '设备',
      icon: '<path d="M12 3l7 3v6c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6z"/><path d="M9.5 12l1.8 1.8L15 10"/>',
    },
    {
      id: 'audit',
      label: '审计',
      icon: '<path d="M12 8v4l2.5 2.5"/><circle cx="12" cy="12" r="9"/>',
    },
  ];

  // 首屏先问会话：未登录才渲染登录页，避免「先闪一下再跳」。
  $effect(() => {
    api
      .me()
      .then((me) => {
        authed = true;
        username = me.username;
        canonicalize();
      })
      .catch((cause: unknown) => {
        if (cause instanceof ApiError && cause.status === 401) authed = false;
        else {
          authed = false;
          toast(cause instanceof Error ? cause.message : '会话校验失败', 'fail');
        }
      });
  });

  /** `/login` 与 `/console` 是同一份 SPA（docs/20 §4.1）。登录成功后把地址栏
   *  换成 `/console`：否则刷新停在 /login，复制链接给同事还要先走一遍登录页，
   *  而此时他已经登录了。 */
  function canonicalize(): void {
    if (window.location.pathname !== '/console') {
      history.replaceState(null, '', '/console');
    }
  }

  function onLoggedIn(name: string): void {
    username = name;
    authed = true;
    canonicalize();
  }

  async function exit(): Promise<void> {
    try {
      await api.logout();
    } catch {
      // 退出是本地决定：即使服务端不可达也要把界面退回登录页。
    }
    authed = false;
    history.replaceState(null, '', '/login');
  }
</script>

{#if authed === null}
  <div class="grid min-h-dvh place-items-center text-sm text-muted">加载中…</div>
{:else if !authed}
  <Login {onLoggedIn} />
{:else}
  <div
    class="grid h-dvh grid-cols-[208px_1fr] overflow-hidden max-[900px]:h-auto max-[900px]:grid-cols-1 max-[900px]:overflow-visible"
  >
    <aside
      class="flex h-dvh flex-col border-r border-line bg-surface px-3 py-4 max-[900px]:static max-[900px]:h-auto max-[900px]:flex-row max-[900px]:items-center max-[900px]:gap-3"
    >
      <div class="flex items-center gap-2.5">
        <div
          class="flex h-[34px] w-[34px] items-center justify-center rounded-[9px] bg-gradient-to-br from-accent to-sky-500 text-white"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.2"
            stroke-linecap="round"><path d="M5 12h14M13 6l6 6-6 6" /></svg
          >
        </div>
        <div class="font-semibold">千寻中继</div>
      </div>
      <nav
        class="mt-[18px] flex flex-col gap-0.5 max-[900px]:mt-0 max-[900px]:ml-auto max-[900px]:flex-row max-[900px]:flex-wrap max-[900px]:gap-1"
      >
        {#each nav as item (item.id)}
          <button
            class="flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13.5px] no-underline hover:bg-surface2 hover:text-fg {page ===
            item.id
              ? 'bg-surface2 text-fg shadow-[inset_2px_0_0_var(--color-accent)]'
              : 'text-muted'}"
            onclick={() => (page = item.id)}
            aria-current={page === item.id ? 'page' : undefined}
          >
            <svg
              class="h-4 w-4 shrink-0 max-[900px]:hidden"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              ><!-- eslint-disable-next-line svelte/no-at-html-tags -->
              {@html item.icon}</svg
            >{item.label}
          </button>
        {/each}
        <button
          class="mt-2.5 rounded-lg border-t border-line px-2.5 py-2.5 text-left text-[13.5px] text-muted no-underline hover:bg-surface2 hover:text-fg max-[900px]:mt-0 max-[900px]:border-0 max-[900px]:py-2"
          onclick={exit}
        >
          退出
        </button>
      </nav>
      <div class="mt-auto text-[11.5px] text-muted max-[900px]:hidden">
        v0.2.0 · 自托管 · {username}
      </div>
    </aside>

    <main
      class="h-dvh overflow-y-auto max-w-[1120px] px-7 py-6 max-[900px]:h-auto max-[900px]:overflow-visible max-[900px]:px-4 max-[900px]:py-4"
    >
      {#if page === 'overview'}
        <Overview />
      {:else if page === 'devices'}
        <Devices />
      {:else}
        <Audit />
      {/if}
    </main>
  </div>
{/if}

<Toasts />
