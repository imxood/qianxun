<script lang="ts">
  /**
   * Laya Reflex · 俄罗斯方块实验室。
   *
   * SENSE(棋局快照)→ DECIDE(Laya System-1:choice+置信度门控)→
   * ACT(游戏循环执行)。System 2 = Dellacherie 枚举求解器,在
   * reflex 模式给出目标放置、在 Laya 低置信度时接管(ESCALATE)。
   * 驱动模式:reflex(默认)/ pure(无规划,纯反射)/ solver(对照)/ human(键盘)。
   *
   * 布局约束:整页铺满可视区,无滚动条——canvas 按容器实测高度自适应格子尺寸,
   * 控件全部收进底部横排控制条。
   */

  import { onMount } from 'svelte';
  import {
    COLS,
    ROWS,
    PIECE_IDS,
    PIECE_NAMES,
    cellsOf,
    clearLines,
    collides,
    dropY,
    features,
    makeBag,
    newBoard,
    lockPiece,
    stackHeight,
    type Board,
  } from './engine';
  import {
    LayaBrain,
    atTarget,
    planTarget,
    type Decision,
    type DriveMode,
    type Snapshot,
  } from './brain';

  type Phase = 'idle' | 'running' | 'paused' | 'over';

  // ---- 游戏状态(board 整体替换以触发重绘) ----
  let board = $state<Board>(newBoard());
  let current = $state<{ id: number; rot: number; x: number; y: number } | null>(null);
  let nextPiece = $state(1);
  let phase = $state<Phase>('idle');
  let score = $state(0);
  let lines = $state(0);
  let level = $state(1);
  let pieces = $state(0);

  // ---- 决策状态 ----
  let driveMode = $state<DriveMode>('reflex');
  let intervalMs = $state(150);
  let gravityMs = $state(800);
  let lastDecision = $state<Decision | null>(null);
  let errorText = $state('');
  let log = $state<Array<{ t: string; text: string; cls: string }>>([]);
  let stats = $state({
    qps: 0,
    avgConf: 0,
    avgLat: 0,
    s2: 0,
    reSense: 0,
    escalate: 0,
    decisions: 0,
  });
  let currentPlan = $state<ReturnType<typeof planTarget> | null>(null);

  const brain = new LayaBrain({ mode: 'reflex' });
  const makeNext = makeBag();
  let gravityTimer: ReturnType<typeof setInterval> | undefined;
  let decideTimer: ReturnType<typeof setInterval> | undefined;
  let deciding = false;
  const startedAt = { v: 0 };

  const COLORS: Record<number, string> = {
    1: '#22d3ee',
    2: '#eab308',
    3: '#a855f7',
    4: '#22c55e',
    5: '#ef4444',
    6: '#3b82f6',
    7: '#f97316',
  };

  function snapshot(): Snapshot {
    return {
      board,
      pieceId: current?.id ?? 1,
      pieceRot: current?.rot ?? 0,
      pieceX: current?.x ?? 0,
      pieceY: current?.y ?? 0,
      nextPieceId: nextPiece,
    };
  }

  function addLog(text: string, cls: string): void {
    const t =
      new Date().toLocaleTimeString('zh-CN', { hour12: false }) +
      '.' +
      String(Date.now() % 1000).padStart(3, '0');
    log = [{ t, text, cls }, ...log].slice(0, 60);
  }

  // ---- SENSE 派生 ----
  const sense = $derived.by(() => {
    const f = features(board, 0);
    const h = stackHeight(board);
    const danger = Math.min(100, Math.round(((h / 20) * 0.6 + (f.holes / 12) * 0.4) * 100));
    return { holes: f.holes, bumpiness: colBump(), height: h, danger };
  });

  function colBump(): number {
    const hs: number[] = [];
    for (let x = 0; x < COLS; x += 1) {
      let h = 0;
      for (let y = 0; y < ROWS; y += 1) {
        if (board[y * COLS + x] !== 0) {
          h = ROWS - y;
          break;
        }
      }
      hs.push(h);
    }
    let b = 0;
    for (let i = 0; i < hs.length - 1; i += 1) b += Math.abs((hs[i] ?? 0) - (hs[i + 1] ?? 0));
    return b;
  }

  // ---- 游戏核心 ----
  function spawnNext(): boolean {
    const id = nextPiece;
    nextPiece = makeNext();
    const x = id === PIECE_IDS.O ? 4 : 3;
    if (collides(board, id, 0, x, 0)) {
      current = { id, rot: 0, x, y: 0 };
      stop('over');
      addLog(`GAME OVER score=${score}`, 'text-red-400');
      return false;
    }
    current = { id, rot: 0, x, y: 0 };
    pieces += 1;
    currentPlan = driveMode === 'reflex' ? planTarget(snapshot()) : null;
    return true;
  }

  function lockCurrent(): void {
    if (!current) return;
    lockPiece(board, current.id, current.rot, current.x, current.y);
    const { board: cleared, cleared: n } = clearLines(board);
    board = cleared;
    if (n > 0) {
      lines += n;
      score += ([0, 100, 300, 500, 800][n] ?? 0) * level;
      level = Math.floor(lines / 10) + 1;
    }
    spawnNext();
  }

  function tryMove(dx: number): void {
    if (!current) return;
    if (!collides(board, current.id, current.rot, current.x + dx, current.y)) current.x += dx;
  }

  function tryRotate(dir: number): void {
    if (!current) return;
    const rot = (current.rot + dir + 4) % 4;
    for (const kick of [0, -1, 1, -2, 2]) {
      if (!collides(board, current.id, rot, current.x + kick, current.y)) {
        current.rot = rot;
        current.x += kick;
        return;
      }
    }
  }

  function softDrop(): void {
    if (!current) return;
    if (!collides(board, current.id, current.rot, current.x, current.y + 1)) current.y += 1;
    else lockCurrent();
  }

  function hardDrop(): void {
    if (!current) return;
    current.y = dropY(board, current.id, current.rot, current.x);
    lockCurrent();
  }

  function applyAction(action: string): void {
    switch (action) {
      case 'shift_left':
        tryMove(-1);
        break;
      case 'shift_right':
        tryMove(1);
        break;
      case 'rotate':
        tryRotate(1);
        break;
      case 'rotate_ccw':
        tryRotate(-1);
        break;
      case 'soft_drop':
        softDrop();
        break;
      case 'hard_drop':
        hardDrop();
        break;
      default:
        break;
    }
  }

  /** ESCALATE:System 2 直接把 piece 放到最优位(瞬移锁定)。 */
  function escalatePlace(): void {
    if (!current) return;
    const best = planTarget(snapshot())?.placement ?? null;
    if (!best || !Number.isFinite(best.score)) {
      hardDrop();
      return;
    }
    current.rot = best.rot;
    current.x = best.x;
    hardDrop();
  }

  // ---- 决策循环 ----
  async function decideOnce(): Promise<void> {
    if (deciding || phase !== 'running' || !current) return;
    deciding = true;
    try {
      errorText = '';
      const snap = snapshot();
      const d: Decision = await brain.decide(snap);
      lastDecision = d;

      if (d.gate === 'ESCALATE') {
        stats.escalate += 1;
        stats.s2 += 1;
        escalatePlace();
        addLog(`ESCALATE → S2 place (${d.latencyMs.toFixed(0)}ms)`, 'text-red-400');
      } else {
        let acted = d.action;
        // reflex 模式下已对齐目标则一步 hard_drop 收尾(语义等价,不悬停)
        if (driveMode === 'reflex' && atTarget(snap, currentPlan) && d.action !== 'hard_drop') {
          acted = 'hard_drop';
        }
        applyAction(acted);
        addLog(
          `EXEC ${acted} conf=${d.conf.toFixed(2)} (${d.latencyMs.toFixed(1)}ms)`,
          d.gate === 'EXECUTE' ? 'text-emerald-400' : 'text-amber-400',
        );
        if (d.sensed > 1) stats.reSense += 1;
      }

      stats.decisions += 1;
      const n = stats.decisions;
      stats.avgConf = stats.avgConf + (d.conf - stats.avgConf) / n;
      stats.avgLat = stats.avgLat + (d.latencyMs - stats.avgLat) / n;
      stats.qps = Math.round((n / Math.max(1, (Date.now() - startedAt.v) / 1000)) * 10) / 10;
    } catch (e) {
      errorText = e instanceof Error ? e.message : String(e);
    } finally {
      deciding = false;
    }
  }

  // ---- 控制与计时 ----
  function applyTimers(): void {
    clearInterval(gravityTimer);
    clearInterval(decideTimer);
    gravityTimer = setInterval(() => {
      if (phase === 'running') softDrop();
    }, gravityMs);
    // 人玩不跑决策循环;Laya 模式按滑杆频率感知
    if (driveMode !== 'human') {
      decideTimer = setInterval(() => void decideOnce(), intervalMs);
    }
  }

  function start(): void {
    if (phase === 'running') return;
    if (phase === 'over' || phase === 'idle') {
      board = newBoard();
      score = 0;
      lines = 0;
      level = 1;
      pieces = 0;
      nextPiece = makeNext();
      stats = { qps: 0, avgConf: 0, avgLat: 0, s2: 0, reSense: 0, escalate: 0, decisions: 0 };
      log = [];
      lastDecision = null;
      brain.mode = driveMode;
      brain.reset();
      startedAt.v = Date.now();
      spawnNext();
    }
    phase = 'running';
    applyTimers();
  }

  function pause(): void {
    if (phase !== 'running') return;
    phase = 'paused';
    clearInterval(gravityTimer);
    clearInterval(decideTimer);
  }

  function restart(): void {
    stop('idle');
    start();
  }

  function stop(to: Phase): void {
    phase = to;
    clearInterval(gravityTimer);
    clearInterval(decideTimer);
  }

  /** 模式/参数变化:运行中即时重排计时器,并刷新规划目标。 */
  function onModeChange(): void {
    brain.mode = driveMode;
    currentPlan = driveMode === 'reflex' && current ? planTarget(snapshot()) : null;
    if (phase === 'running') applyTimers();
  }

  // ---- 人玩键盘 ----
  function onKey(e: KeyboardEvent): void {
    if (driveMode !== 'human' || phase !== 'running') return;
    switch (e.key) {
      case 'ArrowLeft':
        tryMove(-1);
        e.preventDefault();
        break;
      case 'ArrowRight':
        tryMove(1);
        e.preventDefault();
        break;
      case 'ArrowDown':
        softDrop();
        e.preventDefault();
        break;
      case 'ArrowUp':
        tryRotate(1);
        e.preventDefault();
        break;
      case ' ':
        hardDrop();
        e.preventDefault();
        break;
      default:
        break;
    }
  }

  onMount(() => {
    nextPiece = makeNext();
    return () => {
      clearInterval(gravityTimer);
      clearInterval(decideTimer);
    };
  });

  // ---- 渲染:格子尺寸随容器实测自适应(满幅、无滚动) ----
  let boardWrap = $state<HTMLDivElement>();
  let cell = $state(24);

  $effect(() => {
    const el = boardWrap;
    if (!el) return;
    const update = () => {
      const byH = Math.floor((el.clientHeight - 6) / ROWS);
      const byW = Math.floor((el.clientWidth - 6) / COLS);
      cell = Math.max(12, Math.min(30, Math.min(byH, byW)));
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  });

  let boardCanvas = $state<HTMLCanvasElement>();
  let nextCanvas = $state<HTMLCanvasElement>();
  const NEXT_CELL = 16;

  function drawBoard(ctx: CanvasRenderingContext2D): void {
    ctx.clearRect(0, 0, COLS * cell, ROWS * cell);
    ctx.strokeStyle = 'rgba(128,128,128,0.18)';
    for (let x = 0; x <= COLS; x += 1) {
      ctx.beginPath();
      ctx.moveTo(x * cell, 0);
      ctx.lineTo(x * cell, ROWS * cell);
      ctx.stroke();
    }
    for (let y = 0; y <= ROWS; y += 1) {
      ctx.beginPath();
      ctx.moveTo(0, y * cell);
      ctx.lineTo(COLS * cell, y * cell);
      ctx.stroke();
    }
    for (let y = 0; y < ROWS; y += 1) {
      for (let x = 0; x < COLS; x += 1) {
        const v = board[y * COLS + x] ?? 0;
        if (v !== 0) {
          ctx.fillStyle = COLORS[v] ?? '#888';
          ctx.fillRect(x * cell + 1, y * cell + 1, cell - 2, cell - 2);
        }
      }
    }
    if (current) {
      ctx.fillStyle = COLORS[current.id] ?? '#888';
      for (const [dx, dy] of cellsOf(current.id, current.rot)) {
        ctx.fillRect((current.x + dx) * cell + 1, (current.y + dy) * cell + 1, cell - 2, cell - 2);
      }
      // 规划目标虚线(reflex 模式,对应 JEV Lab 的 magenta 意图框)
      if (
        driveMode === 'reflex' &&
        currentPlan?.placement &&
        Number.isFinite(currentPlan.placement.score)
      ) {
        const p = currentPlan.placement;
        ctx.strokeStyle = '#e879f9';
        ctx.setLineDash([4, 3]);
        for (const [dx, dy] of cellsOf(current.id, p.rot)) {
          ctx.strokeRect((p.x + dx) * cell + 2, (p.y + dy) * cell + 2, cell - 4, cell - 4);
        }
        ctx.setLineDash([]);
      }
    }
  }

  function drawNext(ctx: CanvasRenderingContext2D): void {
    ctx.clearRect(0, 0, 4 * NEXT_CELL, 3 * NEXT_CELL);
    ctx.fillStyle = COLORS[nextPiece] ?? '#888';
    for (const [dx, dy] of cellsOf(nextPiece, 0)) {
      ctx.fillRect(dx * NEXT_CELL + 4, dy * NEXT_CELL + 3, NEXT_CELL - 5, NEXT_CELL - 5);
    }
  }

  $effect(() => {
    void board;
    void current;
    void driveMode;
    void currentPlan;
    void pieces;
    void cell;
    const ctx = boardCanvas?.getContext('2d');
    if (ctx) drawBoard(ctx);
  });

  $effect(() => {
    void nextPiece;
    const ctx = nextCanvas?.getContext('2d');
    if (ctx) drawNext(ctx);
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="flex h-full min-h-0 flex-col gap-3">
  <!-- 标题 + 控制 -->
  <div class="flex items-start justify-between gap-4">
    <div class="min-w-0">
      <h1 class="text-base font-semibold">游戏 · Laya Reflex 俄罗斯方块</h1>
      <p class="truncate text-sm text-muted">
        System 1 反射(Laya)× System 2 规划(Dellacherie 求解器)— State → Choice → Confidence
      </p>
    </div>
    <div class="flex shrink-0 items-center gap-2">
      <button
        class="rounded-lg bg-emerald-500/90 px-4 py-2 text-sm font-medium text-white hover:bg-emerald-500 disabled:opacity-40"
        onclick={start}
        disabled={phase === 'running'}
      >
        ▶ 开始
      </button>
      <button
        class="rounded-lg border border-line px-4 py-2 text-sm hover:bg-accent-soft/50 disabled:opacity-40"
        onclick={pause}
        disabled={phase !== 'running'}
      >
        ⏸ 暂停
      </button>
      <button
        class="rounded-lg border border-line px-4 py-2 text-sm hover:bg-accent-soft/50"
        onclick={restart}
      >
        ↺ 重开
      </button>
    </div>
  </div>

  {#if errorText}
    <div class="rounded-lg border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-400">
      laya-server 不可达:{errorText}(先启动:laya-server --model-dir models\laya-multilingual-onnx -p
      10230 --threads 8)
    </div>
  {/if}

  <!-- 三栏:SENSE | DECIDE | ACT -->
  <div class="grid min-h-0 flex-1 grid-cols-12 gap-3">
    <!-- 01 SENSE -->
    <section
      class="col-span-3 min-h-0 overflow-y-auto rounded-xl border border-line bg-surface p-4"
    >
      <h2 class="mb-3 text-xs font-semibold tracking-wider text-muted">01 SENSE → STATE</h2>
      <dl class="space-y-2 text-sm">
        <div class="flex justify-between">
          <dt class="text-muted">falling piece</dt>
          <dd class="font-mono">
            {#if current}{PIECE_NAMES[current.id - 1]} · rot {current.rot} · x {current.x}{:else}-{/if}
          </dd>
        </div>
        <div class="flex justify-between">
          <dt class="text-muted">next</dt>
          <dd class="font-mono">{PIECE_NAMES[nextPiece - 1]}</dd>
        </div>
        <div class="flex justify-between">
          <dt class="text-muted">stack height</dt>
          <dd class="font-mono">{sense.height} / 20</dd>
        </div>
        <div class="flex justify-between">
          <dt class="text-muted">holes</dt>
          <dd class="font-mono">{sense.holes}</dd>
        </div>
        <div class="flex justify-between">
          <dt class="text-muted">bumpiness</dt>
          <dd class="font-mono">{sense.bumpiness}</dd>
        </div>
        <div>
          <div class="mb-1 flex justify-between">
            <dt class="text-muted">DANGER</dt>
            <dd class="font-mono">{sense.danger}%</dd>
          </div>
          <div class="h-1.5 overflow-hidden rounded bg-line">
            <div class="h-full bg-emerald-500" style={`width:${sense.danger}%`}></div>
          </div>
        </div>
        {#if driveMode === 'reflex' && currentPlan?.placement && Number.isFinite(currentPlan.placement.score)}
          <div
            class="rounded-lg border border-fuchsia-500/30 bg-fuchsia-500/5 p-2 font-mono text-xs text-fuchsia-400"
          >
            S2 target: {PIECE_NAMES[current?.id ?? 1]} → rot {currentPlan.placement.rot}, x
            {currentPlan.placement.x}
          </div>
        {/if}
      </dl>
    </section>

    <!-- 02 DECIDE -->
    <section class="col-span-5 flex min-h-0 flex-col gap-3">
      <div class="rounded-xl border border-line bg-surface p-4">
        <div class="mb-2 flex items-center justify-between">
          <h2 class="text-xs font-semibold tracking-wider text-muted">02 DECIDE → LAYA REFLEX</h2>
          <span class="text-xs text-muted">{stats.qps} QPS</span>
        </div>
        <div class="flex items-center gap-4">
          <div class="font-mono text-4xl font-bold text-emerald-400">
            {lastDecision ? (lastDecision.conf * 100).toFixed(1) : '--'}<span class="text-base"
              >%</span
            >
          </div>
          <div class="min-w-0 flex-1">
            {#if lastDecision}
              <span
                class="inline-block rounded px-2 py-0.5 text-xs font-semibold {lastDecision.gate ===
                'EXECUTE'
                  ? 'bg-emerald-500/20 text-emerald-400'
                  : lastDecision.gate === 'RE_SENSE'
                    ? 'bg-amber-500/20 text-amber-400'
                    : 'bg-red-500/20 text-red-400'}"
              >
                {lastDecision.gate}
              </span>
              <span class="ml-2 font-mono text-xs break-all text-muted">
                {lastDecision.action} · {lastDecision.latencyMs.toFixed(1)}ms · sense #{lastDecision.sensed}
              </span>
            {:else}
              <span class="text-sm text-muted">等待开始…</span>
            {/if}
          </div>
        </div>
        {#if lastDecision && Object.keys(lastDecision.probs).length > 0}
          <div class="mt-2 grid grid-cols-2 gap-x-4 gap-y-0.5">
            {#each Object.entries(lastDecision.probs) as [action, p] (action)}
              <div class="flex items-center gap-2 text-xs">
                <span class="w-20 shrink-0 font-mono text-muted">{action}</span>
                <div class="h-1.5 min-w-0 flex-1 overflow-hidden rounded bg-line">
                  <div
                    class="h-full bg-emerald-500"
                    style={`width:${Math.min(100, (Number(p) || 0) * 100)}%`}
                  ></div>
                </div>
                <span class="w-9 text-right font-mono">{Number(p).toFixed(2)}</span>
              </div>
            {/each}
          </div>
        {/if}
        <p class="mt-2 text-xs text-muted">
          conf ≥ {brain.gateExecute.toFixed(2)} EXECUTE ·
          {brain.gateEscalate.toFixed(2)}~{brain.gateExecute.toFixed(2)} RE_SENSE · &lt;
          {brain.gateEscalate.toFixed(2)} ESCALATE(System 2)
        </p>
      </div>

      <div class="grid grid-cols-3 gap-2">
        {#each [['AVG CONF', stats.avgConf.toFixed(2)], ['AVG LAT', `${stats.avgLat.toFixed(0)}ms`], ['ESCALATE', String(stats.escalate)], ['RE-SENSE', String(stats.reSense)], ['S2 PLANS', String(stats.s2)], ['决策数', String(stats.decisions)]] as [label, value] (label)}
          <div class="rounded-lg border border-line bg-surface px-3 py-2">
            <div class="font-mono text-base font-semibold">{value}</div>
            <div class="text-xs text-muted">{label}</div>
          </div>
        {/each}
      </div>

      <div class="min-h-0 flex-1 overflow-y-auto rounded-xl border border-line bg-surface p-3">
        <h3 class="mb-2 text-xs font-semibold tracking-wider text-muted">DECISION STREAM</h3>
        <div class="space-y-0.5 font-mono text-xs">
          {#each log as entry (entry.t + entry.text)}
            <div class={entry.cls}>
              <span class="text-muted">{entry.t}</span>
              {entry.text}
            </div>
          {:else}
            <div class="text-muted">—</div>
          {/each}
        </div>
      </div>
    </section>

    <!-- 03 ACT -->
    <section class="col-span-4 flex min-h-0 flex-col rounded-xl border border-line bg-surface p-4">
      <h2 class="mb-3 text-xs font-semibold tracking-wider text-muted">03 ACT → GAME LOOP</h2>
      <div class="flex min-h-0 flex-1 gap-3">
        <div bind:this={boardWrap} class="flex min-h-0 min-w-0 flex-1 items-start justify-center">
          <canvas
            bind:this={boardCanvas}
            width={COLS * cell}
            height={ROWS * cell}
            class="rounded-lg border border-line bg-black/40"
          ></canvas>
        </div>
        <div class="flex w-28 shrink-0 flex-col gap-2">
          <div class="rounded-lg border border-line p-2 text-center">
            <div class="font-mono text-xl font-bold text-emerald-400">{score}</div>
            <div class="text-xs text-muted">SCORE</div>
          </div>
          <div class="rounded-lg border border-line p-2 text-center">
            <div class="font-mono text-xl font-bold">{lines}</div>
            <div class="text-xs text-muted">LINES · LV {level}</div>
          </div>
          <div class="rounded-lg border border-line p-2">
            <canvas
              bind:this={nextCanvas}
              width={4 * NEXT_CELL}
              height={3 * NEXT_CELL}
              class="mx-auto"
            ></canvas>
            <div class="text-center text-xs text-muted">NEXT</div>
          </div>
        </div>
      </div>
      {#if phase === 'over'}
        <div class="mt-2 rounded-lg bg-red-500/15 py-2 text-center">
          <span class="font-semibold text-red-400">GAME OVER</span>
          <span class="ml-2 font-mono text-muted">score {score} · lines {lines}</span>
        </div>
      {/if}
    </section>
  </div>

  <!-- 底部控制条(横跨,不挤占棋盘高度) -->
  <div
    class="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-xl border border-line bg-surface px-4 py-2.5"
  >
    <div class="flex items-center gap-1">
      {#each [['reflex', 'Laya 反射'], ['pure', '纯反射'], ['solver', 'System 2'], ['human', '人玩']] as [m, label] (m)}
        <button
          class="rounded-lg border px-2.5 py-1 text-xs {driveMode === m
            ? 'border-emerald-500 bg-emerald-500/10 text-emerald-400'
            : 'border-line text-muted hover:bg-accent-soft/50'}"
          onclick={() => {
            driveMode = m as DriveMode;
            onModeChange();
          }}
        >
          {label}
        </button>
      {/each}
    </div>
    <label class="flex items-center gap-2 text-xs text-muted">
      决策 {intervalMs}ms
      <input
        type="range"
        min="80"
        max="500"
        step="10"
        bind:value={intervalMs}
        class="w-28 accent-emerald-500"
        onchange={onModeChange}
      />
    </label>
    <label class="flex items-center gap-2 text-xs text-muted">
      重力 {gravityMs}ms
      <input
        type="range"
        min="300"
        max="2000"
        step="50"
        bind:value={gravityMs}
        class="w-28 accent-emerald-500"
      />
    </label>
    <label class="flex items-center gap-2 text-xs text-muted">
      EXEC ≥ {brain.gateExecute.toFixed(2)}
      <input
        type="range"
        min="0.05"
        max="0.9"
        step="0.01"
        bind:value={brain.gateExecute}
        class="w-28 accent-emerald-500"
      />
    </label>
    <label class="flex items-center gap-2 text-xs text-muted">
      ESCALATE &lt; {brain.gateEscalate.toFixed(2)}
      <input
        type="range"
        min="0.01"
        max="0.5"
        step="0.01"
        bind:value={brain.gateEscalate}
        class="w-24 accent-emerald-500"
      />
    </label>
    <span class="ml-auto text-xs text-muted">
      {driveMode === 'human'
        ? '人玩:←→ 移动 · ↑ 旋转 · ↓ 软降 · 空格 硬降'
        : driveMode === 'solver'
          ? 'System 2 求解器直控(对照基线)'
          : driveMode === 'pure'
            ? '纯反射:无规划目标,7 动作自由选择'
            : '反射:约束解码——Laya 在朝向 S2 目标的合法动作集中选择'}
    </span>
  </div>
</div>
