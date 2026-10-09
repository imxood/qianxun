<script lang="ts">
  import { api, ApiError } from '../lib/api';

  let { onLoggedIn }: { onLoggedIn: (username: string) => void } = $props();

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let submitting = $state(false);

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (submitting) return;
    error = '';
    submitting = true;
    try {
      const result = await api.login(username, password);
      onLoggedIn(result.username);
    } catch (cause) {
      // 锁定时给剩余时间（docs/20 §4.9）：让重试节奏可预期，而不是干等。
      error =
        cause instanceof ApiError && cause.retryAfter
          ? `尝试过于频繁，请 ${cause.retryAfter} 秒后再试`
          : cause instanceof Error
            ? cause.message
            : '登录失败';
      password = '';
    } finally {
      submitting = false;
    }
  }
</script>

<section class="grid min-h-dvh place-items-center p-4">
  <form class="w-[min(92vw,360px)] rounded-2xl border border-line bg-surface p-8" onsubmit={submit}>
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
    <h1 class="my-[18px] text-lg font-semibold">登录控制台</h1>

    {#if error}
      <div
        class="mb-3.5 rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-[13px] text-danger"
        role="alert"
      >
        {error}
      </div>
    {/if}

    <div class="mb-3.5">
      <label for="u" class="mb-1.5 block text-xs text-muted">用户名</label>
      <input
        id="u"
        name="username"
        autocomplete="username"
        bind:value={username}
        class="focus-ring w-full rounded-lg border border-line bg-bg px-3 py-[11px] text-[15px] outline-none focus:border-accent"
      />
    </div>
    <div class="mb-3.5">
      <label for="p" class="mb-1.5 block text-xs text-muted">密码</label>
      <input
        id="p"
        name="password"
        type="password"
        autocomplete="current-password"
        bind:value={password}
        class="focus-ring w-full rounded-lg border border-line bg-bg px-3 py-[11px] text-[15px] outline-none focus:border-accent"
      />
    </div>
    <button
      class="btn btn-primary focus-ring w-full"
      type="submit"
      disabled={submitting || !username || !password}
    >
      {submitting ? '登录中…' : '登录'}
    </button>
    <p class="mt-4 text-center text-xs leading-[1.9] text-muted">
      手机配对？<a href="/qx-pair" class="text-info no-underline">打开配对页</a>
    </p>
  </form>
</section>
