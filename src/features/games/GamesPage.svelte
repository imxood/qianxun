<script lang="ts">
  /**
   * 游戏中心:游戏列表 + 详情切换。
   * 新游戏在这里注册一行即可进入列表。
   */

  import TetrisLab from './tetris/TetrisLab.svelte';

  type GameId = 'laya-tetris';

  const GAMES: Array<{ id: GameId; name: string; tag: string; desc: string }> = [
    {
      id: 'laya-tetris',
      name: 'Laya Reflex · 俄罗斯方块',
      tag: 'System 1 × System 2',
      desc: '完全由 Laya 驱动的俄罗斯方块实验室:反射模型选动作、置信度门控三分支(EXECUTE / RE-SENSE / ESCALATE),Dellacherie 求解器兜底。需要 laya-server(127.0.0.1:10230)。',
    },
  ];

  let active = $state<GameId | null>(null);
</script>

{#if active === null}
  <div class="flex h-full flex-col gap-4">
    <div>
      <h1 class="qx-page-title text-base font-semibold">游戏</h1>
      <p class="text-sm text-muted">
        Laya 决策引擎的游乐场 — 每个游戏都是一次 System 1 能力的实机验证。
      </p>
    </div>
    <div class="grid grid-cols-2 gap-4 lg:grid-cols-3">
      {#each GAMES as game (game.id)}
        <button
          class="group rounded-xl border border-line bg-surface p-5 text-left transition-all hover:border-emerald-500/50 hover:bg-accent-soft/30"
          onclick={() => (active = game.id)}
        >
          <div
            class="mb-2 inline-block rounded bg-emerald-500/10 px-2 py-0.5 text-xs text-emerald-400"
          >
            {game.tag}
          </div>
          <div class="text-base font-semibold">{game.name}</div>
          <p class="mt-1 line-clamp-3 text-sm text-muted">{game.desc}</p>
          <div
            class="mt-3 text-xs text-emerald-400 opacity-0 transition-opacity group-hover:opacity-100"
          >
            进入 →
          </div>
        </button>
      {/each}
    </div>
  </div>
{:else if active === 'laya-tetris'}
  <div class="flex h-full flex-col gap-3">
    <button
      class="self-start rounded-lg border border-line px-3 py-1.5 text-sm text-muted hover:bg-accent-soft/50"
      onclick={() => (active = null)}
    >
      ← 返回游戏列表
    </button>
    <div class="min-h-0 flex-1">
      <TetrisLab />
    </div>
  </div>
{/if}
