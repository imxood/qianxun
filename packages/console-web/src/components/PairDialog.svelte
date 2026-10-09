<script lang="ts">
  import { api } from '../lib/api';
  import { countdown } from '../lib/fmt';
  import { qrSvgPath } from '../lib/qr';

  let { open = $bindable(false), onPaired }: { open?: boolean; onPaired: () => void } = $props();

  type Phase =
    | { kind: 'pending'; code: string; expiresAt: number }
    | { kind: 'used'; deviceName: string }
    | { kind: 'expired' }
    | { kind: 'fail'; message: string };

  let phase = $state<Phase>({ kind: 'expired' });
  let remain = $state(0);
  let qr = $state<{ path: string; viewBox: string } | null>(null);
  let copied = $state(false);

  // 状态机（docs/20 §4.9）：待使用(倒计时) → 已使用(显示设备名,自动收起) → 已过期(可重生成)。
  const POLL_MS = 1500;

  $effect(() => {
    if (!open) return;
    void issue();
    const timer = setInterval(() => void tick(), POLL_MS);
    return () => clearInterval(timer);
  });

  async function issue(): Promise<void> {
    try {
      const issued = await api.issuePair();
      qr = qrSvgPath(issued.pairUrl);
      phase = { kind: 'pending', code: issued.code, expiresAt: issued.expiresAt };
      remain = Math.max(0, issued.expiresAt - Math.floor(Date.now() / 1000));
    } catch (cause) {
      phase = { kind: 'fail', message: cause instanceof Error ? cause.message : '发码失败' };
    }
  }

  async function tick(): Promise<void> {
    if (phase.kind !== 'pending') return;
    remain = Math.max(0, phase.expiresAt - Math.floor(Date.now() / 1000));
    if (remain === 0) {
      phase = { kind: 'expired' };
      return;
    }
    try {
      const state = await api.pairStatus(phase.code);
      if (state.status === 'used') {
        phase = { kind: 'used', deviceName: state.deviceName ?? '新设备' };
        onPaired();
        // 终态给 1.5 秒让人看清设备名，然后自己收起。
        setTimeout(() => (open = false), 1500);
      }
    } catch {
      // 轮询失败不打断倒计时：下一拍重试，用户看到的是码仍在计时。
    }
  }

  async function copyCode(code: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(code);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      copied = false;
    }
  }
</script>

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-[rgba(7,10,16,.6)] p-4 backdrop-blur-[6px]"
    role="dialog"
    aria-modal="true"
    aria-label="新增设备"
    tabindex="-1"
    onclick={(e) => {
      if (e.target === e.currentTarget) open = false;
    }}
    onkeydown={(e) => {
      if (e.key === 'Escape') open = false;
    }}
  >
    <div
      class="w-[min(94vw,400px)] rounded-2xl border border-line-strong bg-surface shadow-[0_8px_24px_rgba(0,0,0,.45)]"
    >
      <header class="flex items-center justify-between border-b border-line px-5 py-4">
        <h2 class="text-[15px] font-semibold">新增设备</h2>
        <button
          class="btn btn-ghost btn-sm focus-ring"
          aria-label="关闭"
          onclick={() => (open = false)}
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12" /></svg
          >
        </button>
      </header>

      <div class="p-5">
        {#if phase.kind === 'fail'}
          <p class="py-6 text-center text-[13px] text-danger">{phase.message}</p>
        {:else if phase.kind === 'expired'}
          <div class="py-6 text-center">
            <div class="text-[13px] text-warn">配对码已过期</div>
            <button class="btn btn-ghost btn-sm focus-ring mt-4" onclick={issue}>重新生成</button>
          </div>
        {:else if phase.kind === 'used'}
          <div class="py-6 text-center">
            <span class="dot dot-on"></span>
            <div class="mt-3 text-[15px] font-semibold">{phase.deviceName} 已配对</div>
          </div>
        {:else if phase.kind === 'pending'}
          {@const code = phase.code}
          <div class="flex items-center gap-5">
            {#if qr}
              <svg
                viewBox={qr.viewBox}
                width="148"
                height="148"
                role="img"
                aria-label="配对二维码"
                shape-rendering="crispEdges"
                class="h-[148px] w-[148px] shrink-0 rounded-xl bg-white"
              >
                <path fill="#0b0f17" d={qr.path}></path>
              </svg>
            {/if}
            <div class="flex-1">
              <div class="text-[13px] text-muted">扫码或输入配对码</div>
              <!-- 12 位码在 190px 栏里：22px/4px 字距会被裁掉最后一个字符，
                   而少一位的码是配不上的——宁可小一点也要完整。 -->
              <button
                class="mt-2.5 w-full rounded-[10px] border border-dashed border-line-strong bg-bg px-1.5 py-2.5 text-center font-mono text-[17px] leading-none font-semibold tracking-[1.5px] tabular-nums"
                title="点击复制"
                onclick={() => copyCode(code)}
              >
                {code}
              </button>
              <div class="mt-2 text-xs text-muted">{copied ? '已复制' : '点击复制'}</div>
            </div>
          </div>
          <div class="mt-3 flex items-center gap-2 text-xs text-warn">
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></svg
            >
            <span class="font-mono tabular-nums">{countdown(remain)}</span> 后过期
          </div>
        {/if}
      </div>

      <footer class="flex justify-end gap-2.5 border-t border-line px-5 py-3.5">
        {#if phase.kind === 'pending'}
          <button class="btn btn-ghost btn-sm focus-ring" onclick={issue}>重新生成</button>
        {/if}
        <button class="btn btn-primary btn-sm focus-ring" onclick={() => (open = false)}
          >完成</button
        >
      </footer>
    </div>
  </div>
{/if}
