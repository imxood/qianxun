<script lang="ts">
  /**
   * Laya Jump · 超级马里奥 1-1 —— 剧场式舞台 UI(docs/11 §7)。
   *
   * 布局:顶栏 40px + 舞台(占满全部剩余)+ 进度条 3px + 遥测条。
   * 遥测降维为单行摘要 + 点击弹出玻璃浮层(LAYA 决策 / 参数设置);
   * HUD 与 Laya 叠加画进 canvas,不占 DOM 布局。舞台可用面积 ≈92%。
   * 服务约定(docs/11 §1.1):只读探活,绝不代启 laya/qwen——
   * reflex/pure 需 release 版环境页先启动 laya-server。
   */

  import { onMount } from 'svelte';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';

  import FloatingPanel from '../../../lib/ui/FloatingPanel.svelte';
  import type { PostmortemReport, SessionInput } from './postmortem';
  import { DEFAULT_POLICY, getPolicy, setPolicy, type PolicyProfile } from './policy';
  import {
    buildDeathPayload,
    refineViaQwen,
    type DeathDiagnosis,
    type HistoryRow,
    type LayaInsight,
  } from './qwen';
  import {
    CANDIDATE_FILE,
    DESIGN_EVAL_RUNS,
    EVOLUTION_FILE,
    HISTORY_FILE,
    INSIGHTS_FILE,
    PLAYBOOK_FILE,
    POLICY_FILE,
    RUNS_INDEX_FILE,
    STATE_FILE,
    applyCooldowns,
    appendHistory,
    appendInsight,
    auditLine,
    detectRegression,
    diffPatch,
    initialEvoState,
    judgeCandidate,
    limitPatch,
    markAnalyzed,
    noteProposals,
    obsSnapshotOf,
    parseAuditLines,
    parseEvoState,
    parseHistory,
    parsePolicyFile,
    planAnalyses,
    reasonShort,
    recordRun,
    restoreVersion,
    runScore,
    startCooldowns,
    wrapPolicy,
    type AuditEntry,
    type EvoState,
    type QwenCounters,
    type RunIndexRow,
  } from './evolution';
  import {
    CircuitBreaker,
    nextTask,
    removeTask,
    scheduleTask,
    taskFailed,
    type SchedTask,
  } from './machine';
  import { getMarioStore } from './store';

  import { MarioDriver, type LogRow, type SentRow } from './driver';
  import { createGameState, IDLE_INPUT, step, type GameState, type Input } from './engine';
  import { autopilotInput, nextIntent, type Action } from './planner';
  import { buildLevelCanvas, drawFrame } from './renderer';
  import type { DriveMode } from './brain';
  import { COLS, TILE, WORLD_1_1 } from './world1-1';

  let { onBack }: { onBack: () => void } = $props();

  // ---- 引擎世界($state.raw:重赋值可触发更新;深变更刻意不跟踪——
  // 60fps 数据不进深响应式,HUD 走 10Hz 快照,docs/11 §2 差异①) ----
  const world = WORLD_1_1;
  let s = $state.raw<GameState>(createGameState(world));
  const driver = new MarioDriver();
  let level: HTMLCanvasElement | null = null;
  let currentInput: Input = IDLE_INPUT;

  // ---- 响应式:HUD 10Hz 快照 + 驾驶舱状态 ----
  type Phase = 'idle' | 'running' | 'paused';
  let phase = $state<Phase>('idle');
  let mode = $state<DriveMode>('human');
  let hud = $state({ score: 0, coins: 0, time: 300, life: 1, pct: 0 });
  let layaOk = $state(false);
  let qwenOk = $state(false);
  let lastDecision = $state<{
    action: Action;
    conf: number;
    gate: string;
    latencyMs: number;
    probs: Array<[Action, number]>;
    note: string;
  } | null>(null);
  let stats = $state({
    decisions: 0,
    exec: 0,
    reSense: 0,
    escalate: 0,
    stale: 0,
    vetoes: 0,
    avgConf: 0,
    avgLatencyMs: 0,
    qps: 0,
  });
  let log = $state<Array<LogRow & { level: 'game' | 'evolve' | 'error' }>>([]);
  let logFilter = $state<'all' | 'game' | 'evolve' | 'error'>('all');
  let deaths = $state<Array<{ x: number; cause: string }>>([]);
  /** 多窗叠开(docs/15 §5):面板 id 集合,Escape 关最顶层。 */
  type PanelId = 'laya' | 'settings' | 'pm' | 'evo';
  let openPanels = $state<PanelId[]>([]);
  /** 进化干预三态(docs/15 §3.2):自动 / 单步(提案后挂起) / 暂停(只析不判)。 */
  let evolveMode = $state<'auto' | 'step' | 'pause'>('auto');
  let settings = $state({ speed: 1, intervalMs: 250, gateExecute: 0.16, gateEscalate: 0.12 });
  let stateJson = $state('');
  let postmortem = $state<string | null>(null);
  let rippleTick: number | null = null;
  /** Qwen 策略手册(滚动上下文,持久于 mario 数据根 playbook.md)。 */
  let playbook = $state('');
  /** Laya 使用经验沉淀(docs/14 §6,持久于 insights.md)。 */
  let insights = $state('');
  let qwenRefining = $state(false);
  let runHistory = $state<HistoryRow[]>([]);
  /** 分析队列状态(docs/15 §6.2):待分析 n · 在途 · 上次耗时。 */
  let queueDepth = $state(0);
  let analyzingKey = $state<string | null>(null);
  let lastAnalysisMs = $state<number | null>(null);
  /** 单步模式挂起:提案已生成,等「下一步 ▸」放行装填。 */
  let stepGated = $state(false);
  /** 分析硬门(docs/15 §6.2 修订):上一局的分析没完成,下一局不得开始。 */
  let analysisGate: Promise<void> | null = null;
  let analysisPending = $state(false);
  let starting = false;
  /** 分析队列内存态(串行;溢出落 evo.pendingAnalyses)。 */
  let taskQueue: SchedTask[] = [];
  let taskBacklog: SchedTask[] = [];
  const breaker = new CircuitBreaker();
  /** 诊断/补丁链(death-repeat 载荷⑥素材,docs/15 §6.1)。 */
  const diagBySig = new SvelteMap<string, DeathDiagnosis[]>();
  const patchBySig = new SvelteMap<string, Record<string, { from: unknown; to: unknown }>>();
  let lastAnomalyLog = 0;

  // ---- 进化闭环(docs/13):champion 持久于数据根;候选全自动(docs/15 §3.2) ----
  let evo = $state<EvoState>(initialEvoState());
  /** 本局策略快照(局内不可变,docs/13 不变量①);null = 尚未开局。 */
  let runPolicy: PolicyProfile | null = null;
  /** 候选:Qwen 提案(自动装填下局试用;单步模式等放行)。 */
  let pendingCandidate = $state<{
    policy: PolicyProfile;
    patch: Record<string, { from: unknown; to: unknown }>;
    issues: string[];
    reason: string;
    kind: 'numeric' | 'design';
  } | null>(null);
  /** 用户点了「下局试用」的候选;开局时转为 trialCandidate。 */
  let trialArmed = $state(false);
  /** 本局正在试用的候选(局后计分判定)。 */
  let trialCandidate: {
    policy: PolicyProfile;
    patch: Record<string, { from: unknown; to: unknown }>;
    kind: 'numeric' | 'design';
  } | null = null;
  /** 审计尾(evolution.jsonl 最近 10 条,新→旧)。 */
  let auditTail = $state<AuditEntry[]>([]);

  const MODES: Array<{ id: DriveMode; label: string }> = [
    { id: 'human', label: '人玩' },
    { id: 'reflex', label: '反射' },
    { id: 'pure', label: '纯反射' },
    { id: 'autopilot', label: '自驾' },
  ];
  const needsLaya = $derived(mode === 'reflex' || mode === 'pure');
  const canStart = $derived(phase !== 'running' && (!needsLaya || layaOk));

  function pushLog(cls: string, text: string, level: 'game' | 'evolve' | 'error' = 'game'): void {
    const d = new Date();
    const t = `${d.toLocaleTimeString('zh-CN', { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`;
    log = [{ t, cls, text, level }, ...log].slice(0, 40);
  }
  function togglePanel(id: PanelId): void {
    openPanels = openPanels.includes(id) ? openPanels.filter((x) => x !== id) : [...openPanels, id];
  }

  // ---- 持久化(数据根 mario/:policy.json 信封 / playbook.md / history.json /
  // state.json / evolution.jsonl;写入失败静默,不影响游戏) ----
  function persistPolicy(source: string): void {
    void getMarioStore()
      .write(POLICY_FILE, JSON.stringify(wrapPolicy(getPolicy(), source), null, 2))
      .catch(() => {});
  }
  function persistPlaybook(): void {
    void getMarioStore()
      .write(PLAYBOOK_FILE, playbook)
      .catch(() => {});
  }
  function persistInsights(): void {
    void getMarioStore()
      .write(INSIGHTS_FILE, insights)
      .catch(() => {});
  }
  function persistHistory(): void {
    void getMarioStore()
      .write(HISTORY_FILE, JSON.stringify(runHistory, null, 2))
      .catch(() => {});
  }
  function persistEvo(): void {
    void getMarioStore()
      .write(STATE_FILE, JSON.stringify(evo, null, 2))
      .catch(() => {});
  }
  /** 候选持久化(docs/15 §9 教训②):重启不丢提案。 */
  function persistCandidate(): void {
    void getMarioStore()
      .write(CANDIDATE_FILE, JSON.stringify({ pending: pendingCandidate, trialArmed }, null, 2))
      .catch(() => {});
  }
  /**
   * 逐局全量快照(docs/15 §9 数据留存):runs/<runId>.json(收发明细+复盘+
   * 策略快照),索引一行进 runs/index.jsonl;超 1MiB 上限则降级存紧凑版。
   */
  async function persistRunSnapshot(input: {
    report: PostmortemReport;
    session: SessionInput;
    sent: SentRow[];
    mode: DriveMode;
    ticks: number;
    tasksPlanned: number;
  }): Promise<void> {
    const s = getMarioStore();
    const won = input.report.outcome === 'win';
    const meta = {
      runId: driver.runId,
      ts: new Date().toISOString(),
      mode: input.mode,
      won,
      ticks: input.ticks,
      maxXCol: input.report.maxXCol,
      attempts: input.report.attempts,
      policy: runPolicy ?? getPolicy(),
      design: obsSnapshotOf(runPolicy ?? getPolicy()),
    };
    const index: RunIndexRow = {
      ts: meta.ts,
      runId: meta.runId,
      mode: meta.mode,
      won,
      ticks: input.ticks,
      maxXCol: input.report.maxXCol,
      attempts: input.report.attempts,
      deaths: input.report.deaths.length,
      tasksPlanned: input.tasksPlanned,
      tasksOk: 0,
    };
    try {
      await s.write(`runs/${meta.runId}.json`, JSON.stringify({ ...meta, ...input }, null, 2));
    } catch {
      // 超 1MiB(极长空转局):丢弃收发明细,保复盘与元数据
      try {
        await s.write(
          `runs/${meta.runId}.json`,
          JSON.stringify({ ...meta, report: input.report, session: input.session }, null, 2),
        );
      } catch {
        return; // 连紧凑版都写不下:放快照留痕,不影响游戏
      }
    }
    await s.append(RUNS_INDEX_FILE, JSON.stringify(index)).catch(() => {});
  }
  function audit(entry: AuditEntry): void {
    auditTail = [entry, ...auditTail].slice(0, 10);
    void getMarioStore()
      .append(EVOLUTION_FILE, auditLine(entry))
      .catch(() => {});
  }

  /** 设置面板数值与 champion 对齐(提交 champion 变更后调用)。 */
  function syncSettingsFromPolicy(p: PolicyProfile): void {
    settings.intervalMs = p.intervalMs;
    settings.gateExecute = p.gateExecute;
    settings.gateEscalate = p.gateEscalate;
    applyDriverConfig();
  }

  // ---- 候选全自动流(docs/15 §3.2):提案→自动装填→局后自动判定;三态干预 ----
  function armTrial(): void {
    trialArmed = true;
    stepGated = false;
    persistCandidate();
    pushLog('text-sky-400', '候选装填,下局试用', 'evolve');
  }
  /** 单步模式放行(「下一步 ▸」)。 */
  function stepAdvance(): void {
    if (stepGated && pendingCandidate) armTrial();
  }
  /** 兜底干预:重置策略(审计 actor:user)。 */
  function resetPolicy(): void {
    const before = getPolicy();
    setPolicy({ ...DEFAULT_POLICY });
    syncSettingsFromPolicy(getPolicy());
    persistPolicy('user:reset');
    pendingCandidate = null;
    trialArmed = false;
    persistCandidate();
    audit({
      v: 1,
      ts: new Date().toISOString(),
      iter: evo.iteration,
      runId: null,
      actor: 'user',
      action: 'rollback',
      reason: '人工重置',
      patch: diffPatch(before, getPolicy()),
    });
    pushLog('text-amber-400', '策略已重置为默认', 'evolve');
  }

  driver.onDecision = (d) => {
    lastDecision = {
      action: d.action,
      conf: d.conf,
      gate: d.gate,
      latencyMs: d.latencyMs,
      probs: Object.entries(d.probs)
        .map(([k, v]) => [k, Number(v)] as [Action, number])
        .sort((a, b) => b[1] - a[1]),
      note: d.note,
    };
    stateJson = d.stateJson;
    rippleTick = s.tick;
    // 决策拍不进日志(docs/15 §5.6:250ms 一条刷屏)——只留 HUD 门控徽标;
    // 异常拍(ESCALATE/stale)同因 5s 限频进 evolve 档。
    if (d.gate !== 'EXECUTE' && Date.now() - lastAnomalyLog >= 5000) {
      lastAnomalyLog = Date.now();
      pushLog(
        d.gate === 'RE_SENSE' ? 'text-amber-400' : 'text-red-400',
        `${d.gate} ${d.action} ${(d.conf * 100).toFixed(0)}%`,
        'evolve',
      );
    }
  };
  driver.onLog = (row: LogRow) =>
    pushLog(row.cls, row.text, row.cls.includes('red') ? 'error' : 'game');

  // ---- 键盘(人玩) ----
  const keys = new SvelteSet<string>();
  function keyInput(): Input {
    return {
      left: keys.has('ArrowLeft') || keys.has('KeyA'),
      right: keys.has('ArrowRight') || keys.has('KeyD'),
      run: keys.has('ShiftLeft') || keys.has('ShiftRight'),
      jump: keys.has('Space') || keys.has('KeyZ') || keys.has('ArrowUp'),
    };
  }
  function onKey(e: KeyboardEvent, down: boolean): void {
    if (down && e.code === 'Escape' && openPanels.length > 0) {
      openPanels = openPanels.slice(0, -1); // 只关最顶层(docs/15 §5)
      return;
    }
    if (down && e.code === 'KeyP') {
      if (phase === 'running') phase = 'paused';
      else if (phase === 'paused') phase = 'running';
      return;
    }
    if (down && e.code === 'Enter') {
      if (s.phase === 'dead')
        s.deadTimer = 1; // 立即重生
      else if (s.phase === 'won') restart();
      return;
    }
    if (mode !== 'human') return;
    const codes = [
      'ArrowLeft',
      'ArrowRight',
      'ArrowUp',
      'Space',
      'KeyA',
      'KeyD',
      'KeyZ',
      'ShiftLeft',
      'ShiftRight',
    ];
    if (codes.includes(e.code)) {
      if (down) keys.add(e.code);
      else keys.delete(e.code);
      e.preventDefault();
    }
  }
  function onKeyDown(e: KeyboardEvent): void {
    onKey(e, true);
  }
  function onKeyUp(e: KeyboardEvent): void {
    onKey(e, false);
  }

  // ---- 画布与视口:背板 = 世界像素(240 高),CSS 铺满舞台(全宽裁定) ----
  let stageEl = $state<HTMLDivElement>();
  let canvasEl = $state<HTMLCanvasElement>();
  let viewTiles = 24;
  let ctx: CanvasRenderingContext2D | null = null;

  $effect(() => {
    const el = stageEl;
    if (!el) return;
    const update = (): void => {
      const tiles = Math.max(16, Math.min(56, Math.round((el.clientWidth / el.clientHeight) * 15)));
      if (tiles !== viewTiles && canvasEl) {
        viewTiles = tiles;
        canvasEl.width = viewTiles * TILE;
        canvasEl.height = 240;
        ctx = canvasEl.getContext('2d');
      }
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  });

  // ---- 主循环:固定步长累加器(60Hz),速度滑杆只改消费速率 ----
  let raf = 0;
  let last = 0;
  let acc = 0;
  let lastHud = 0;
  const TICK = 1000 / 60;

  function frame(now: number): void {
    raf = requestAnimationFrame(frame);
    const dt = Math.min(120, now - last);
    last = now;
    if (phase === 'running' && !document.hidden) {
      acc += dt * settings.speed;
      let steps = 0;
      while (acc >= TICK && steps < 8) {
        acc -= TICK;
        steps += 1;
        const input: Input =
          mode === 'human'
            ? keyInput()
            : mode === 'autopilot'
              ? autopilotInput(s, world, runPolicy ?? undefined)
              : driver.inputFor(s, world);
        currentInput = input;
        const events = step(world, s, input, viewTiles * TILE);
        for (const e of events) {
          if (e.type === 'death') {
            deaths = [...s.deaths];
            pushLog(
              'text-red-400',
              `第${s.attempts}死 ${e.cause === 'pit' ? '掉坑' : e.cause}@${Math.round(s.mario.x / TILE)}`,
              'game',
            );
            driver.event({
              event: 'death',
              cause: e.cause,
              x: Math.round(s.mario.x),
              tick: s.tick,
              attempt: s.attempts,
            });
            // 第 3 次死亡自动出复盘(失败密度过高 = 该局必须被检讨)
            if (s.attempts >= 3 && !postmortem) genPostmortem();
          } else if (e.type === 'win') {
            pushLog('text-emerald-400', `通关 +${e.bonus}`, 'game');
            driver.event({ event: 'win', score: s.score, attempt: s.attempts });
            if (!postmortem) genPostmortem();
            void driver.flush();
          } else if (e.type === 'respawn') {
            driver.event({ event: 'respawn', attempt: s.attempts });
          }
        }
      }
      if (mode === 'reflex' || mode === 'pure') void driver.maybeDecide(s, world, now);
    }
    if (canvasEl && ctx && level) {
      drawFrame(ctx, level, world, s, {
        viewTiles,
        intent: mode === 'reflex' ? nextIntent(s, world) : null,
        heldInput: currentInput,
        rippleAge: rippleTick === null ? null : s.tick - rippleTick,
      });
    }
    if (now - lastHud >= 100) {
      lastHud = now;
      hud = {
        score: s.score,
        coins: s.coinCount,
        time: Math.max(0, s.timeUnits),
        life: s.attempts,
        pct: Math.min(100, Math.round((s.maxX / world.worldWidthPx) * 100)),
      };
      stats = { ...driver.stats };
    }
  }

  // ---- 控制 ----
  function applyDriverConfig(): void {
    driver.configure({
      mode,
      intervalMs: settings.intervalMs,
      gateExecute: settings.gateExecute,
      gateEscalate: settings.gateEscalate,
    });
  }
  async function start(): Promise<void> {
    if (starting) return;
    if (phase === 'running' || !canStart) return;
    // 分析硬门(docs/15 §6.2):上一局失败/通关的分析没出结果,不开下一局。
    // Qwen 不可达时任务重试上限后放弃,门有界自动放行(不卡死)。
    if (analysisGate !== null) {
      pushLog('text-sky-400', '等待 Qwen 分析完成…', 'evolve');
      starting = true;
      try {
        await analysisGate;
      } finally {
        starting = false;
      }
      if ((phase as Phase) === 'running') return; // 等待期间已被其它路径开局
    }
    if (phase === 'idle' || s.phase === 'won') {
      s = createGameState(world);
      deaths = [];
      log = [];
      lastDecision = null;
      rippleTick = null;
      currentInput = IDLE_INPUT;
      postmortem = null;
    }
    driver.reset();
    // 局内策略快照(docs/13 不变量①):试用候选优先,否则 champion;
    // 局内一律读快照,Qwen/设置面板的更新只影响下一局。
    if (trialArmed && pendingCandidate) {
      trialCandidate = pendingCandidate;
      trialArmed = false;
      persistCandidate();
      runPolicy = { ...pendingCandidate.policy };
      pushLog(
        'text-sky-400',
        `本局试用:${Object.keys(pendingCandidate.patch).join(',')}`,
        'evolve',
      );
    } else {
      trialCandidate = null;
      runPolicy = { ...getPolicy() };
    }
    driver.beginRun(runPolicy);
    driver.configure({
      mode,
      intervalMs: runPolicy.intervalMs,
      gateExecute: runPolicy.gateExecute,
      gateEscalate: runPolicy.gateEscalate,
    });
    phase = 'running';
    void driver.startSession({
      encoding: 'v2-obs',
      intervalMs: runPolicy.intervalMs,
      gateExecute: runPolicy.gateExecute,
      gateEscalate: runPolicy.gateEscalate,
      speed: settings.speed,
      viewportTiles: viewTiles,
    });
    pushLog('text-emerald-400', `开始 · ${MODES.find((m) => m.id === mode)?.label ?? mode}`);
  }
  /** 复盘:规则判定 + 固定格式 Markdown,推入会话 JSONL(离线批扫/Qwen 消费)。 */
  function genPostmortem(): void {
    const { markdown, report } = driver.buildPostmortem();
    postmortem = markdown;
    driver.pushPostmortem(markdown);
    pushLog('text-sky-400', '复盘已生成', 'game');
    runHistory = appendHistory(runHistory, {
      iteration: runHistory.length + 1,
      won: s.phase === 'won',
      maxXCol: report.maxXCol,
      ticks: s.tick,
    });
    persistHistory();

    // ① 试用候选局后判定(+ε 滞回;无基线首个候选直接转正,docs/15 §3.2)
    if (trialCandidate !== null) {
      const c = trialCandidate;
      trialCandidate = null;
      const score = runScore({ won: s.phase === 'won', maxXCol: report.maxXCol, ticks: s.tick });
      const window = c.kind === 'design' ? DESIGN_EVAL_RUNS : 1;
      void window; // UI 单局判定;设计变更窗在 e2e 中位评估生效
      if (evo.championScore === null) {
        setPolicy(c.policy);
        syncSettingsFromPolicy(c.policy);
        persistPolicy(`commit:trial@iter-${evo.iteration}`);
        evo = { ...evo, championScore: score };
        evo = {
          ...evo,
          versions: [
            ...evo.versions,
            {
              vid: evo.versions.reduce((m, v) => Math.max(m, v.vid), 0) + 1,
              iter: evo.iteration,
              ts: new Date().toISOString(),
              policy: c.policy,
              patch: c.patch,
              originSigs: [],
              verdict: 'commit',
              score,
            },
          ],
          championVid: evo.versions.length + 1,
        };
        pendingCandidate = null;
        persistCandidate();
        pushLog('text-emerald-400', `候选转正 +${score.toFixed(0)}(无基线)`, 'evolve');
        audit({
          v: 1,
          ts: new Date().toISOString(),
          iter: evo.iteration,
          runId: driver.runId,
          actor: 'system',
          action: 'commit',
          reason: '无基线,候选直接转正',
          patch: c.patch,
          patchKind: c.kind,
          score: { champion: null, candidate: score },
        });
      } else {
        const v = judgeCandidate(evo.championScore, score);
        const base = evo.championScore ?? 1;
        if (v.verdict === 'commit') {
          setPolicy(c.policy);
          syncSettingsFromPolicy(c.policy);
          persistPolicy(`commit:trial@iter-${evo.iteration}`);
          evo = {
            ...evo,
            championScore: score,
            counters: {
              ...evo.counters,
              qwen: { ...evo.counters.qwen, commits: evo.counters.qwen.commits + 1 },
            },
          };
          pendingCandidate = null;
          persistCandidate();
          pushLog(
            'text-emerald-400',
            `转正 ${score.toFixed(0)}(+${((score / base - 1) * 100).toFixed(0)}%)`,
            'evolve',
          );
        } else {
          evo = {
            ...evo,
            cooldowns: startCooldowns(evo.cooldowns, Object.keys(c.patch), evo.iteration),
            counters: {
              ...evo.counters,
              qwen: { ...evo.counters.qwen, rollbacks: evo.counters.qwen.rollbacks + 1 },
            },
          };
          pendingCandidate = null;
          // 证伪写回(docs/15 §9 教训①):把"此路不通"写进手册,防 Qwen
          // 基于被否决的前提继续推理
          playbook = appendInsight(
            playbook,
            evo.iteration,
            `假设已回滚勿重复:${Object.keys(c.patch).join(',')}(${v.reason})`,
            obsSnapshotOf(getPolicy()),
          );
          persistPlaybook();
          pushLog('text-amber-400', `回滚 · 冷却 5 局`, 'evolve');
        }
        persistCandidate();
        audit({
          v: 1,
          ts: new Date().toISOString(),
          iter: evo.iteration,
          runId: driver.runId,
          actor: 'system',
          action: v.verdict,
          reason: v.reason,
          patch: c.patch,
          patchKind: c.kind,
          score: { champion: evo.championScore, candidate: score },
        });
      }
    }

    // ①b 回归守卫(docs/15 §7):连续退化自动回退最优版本
    const reg = detectRegression(runHistory);
    if (reg !== null) {
      const best = [...evo.versions]
        .filter((x) => x.verdict === 'commit')
        .sort((a, b) => b.score - a.score)[0];
      if (best && best.vid !== evo.championVid) {
        const r = restoreVersion(evo, best.vid);
        if (r !== null) {
          evo = r.state;
          setPolicy(r.policy);
          syncSettingsFromPolicy(r.policy);
          persistPolicy(`restore:v${best.vid}`);
          audit({
            v: 1,
            ts: new Date().toISOString(),
            iter: evo.iteration,
            runId: driver.runId,
            actor: 'system',
            action: 'restore',
            reason: `回归,回退 v${best.vid}`,
          });
          pushLog('text-amber-400', `回归守卫:回退 v${best.vid}`, 'evolve');
        }
      }
    }

    // ② 死亡级分析队列(docs/15 §6):planAnalyses → 串行异步处理,不阻塞游戏
    const session: SessionInput = driver.sessionSnapshot();
    const sent: SentRow[] = [...driver.sentLog];
    const tasks = planAnalyses(evo, report, driver.runId);
    for (const t of tasks) {
      const r = scheduleTask([...taskBacklog, ...taskQueue], t);
      taskQueue = r.queue.filter((x) => !taskBacklog.some((b) => b.key === x.key));
      taskBacklog = r.queue.filter((x) => taskBacklog.some((b) => b.key === x.key));
      if (r.overflow !== null) evo = bumpQwen(evo, 'droppedByCap');
      if (r.merged) evo = bumpQwen(evo, 'merged');
    }
    queueDepth = taskQueue.length + taskBacklog.length;
    // 逐局全量快照(docs/15 §9):runs/<runId>.json + 索引行,失败不影响游戏
    void persistRunSnapshot({
      report,
      session,
      sent,
      mode,
      ticks: s.tick,
      tasksPlanned: tasks.length,
    });
    if (tasks.length > 0 && qwenOk) {
      // 分析硬门:挂起 promise,start()/restart() 必须等它 resolve 才能开下一局
      analysisPending = true;
      const run = runAnalysisQueue(report, session, sent, driver.runId).finally(() => {
        analysisGate = null;
        analysisPending = false;
        queueDepth = taskQueue.length + taskBacklog.length;
      });
      analysisGate = run;
      void run;
    } else {
      if (tasks.length > 0) {
        for (const t of tasks) taskQueue = [...taskQueue, t];
      }
      evo = recordRun(evo, report, driver.runId, false, mode);
      persistEvo();
    }
  }

  function bumpQwen(evo: EvoState, k: keyof QwenCounters, n = 1): EvoState {
    return {
      ...evo,
      counters: { ...evo.counters, qwen: { ...evo.counters.qwen, [k]: evo.counters.qwen[k] + n } },
    };
  }

  function insightToText(ins: LayaInsight): string {
    if (ins.kind === 'claim') {
      const tag = `${ins.metric ?? ''}${ins.direction === 'down' ? '↓' : '↑'}${ins.field ? ` ${ins.field}` : ''}`;
      return `${ins.claim} [${tag.trim()}]`;
    }
    return ins.claim;
  }

  /**
   * 分析队列消费(docs/15 §6.2):局间串行、永不丢任务;成功才占幂等键;
   * 死亡级带全量载荷;补丁按 run 级合并成单一候选(单维度);insight 必填。
   */
  async function runAnalysisQueue(
    report: PostmortemReport,
    session: SessionInput,
    sent: SentRow[],
    runId: string,
  ): Promise<void> {
    if (qwenRefining) return; // 已有队列在跑,任务已在队中等待
    qwenRefining = true;
    const champion = getPolicy();
    const obs = obsSnapshotOf(champion);
    const iter = evo.iteration;
    let hadTasks = false;
    let allOk = true;
    const accPatch: Record<string, { from: unknown; to: unknown }> = {};
    const accSigs = new SvelteSet<string>();
    const issues: string[] = [];
    try {
      for (;;) {
        if (breaker.tripped) break;
        const task = nextTask(taskBacklog, taskQueue);
        if (task === null) break;
        taskBacklog = removeTask(taskBacklog, task.key);
        taskQueue = removeTask(taskQueue, task.key);
        queueDepth = taskQueue.length + taskBacklog.length;
        hadTasks = true;
        analyzingKey = task.key;
        const tA = Date.now();
        audit({
          v: 1,
          ts: new Date().toISOString(),
          iter,
          runId,
          actor: 'qwen',
          action: 'analyze',
          phase: 'start',
          key: task.key,
          reason: task.reason,
        });
        try {
          const death =
            task.kind === 'death'
              ? report.deaths.find((d) => d.attempt === task.attempt)
              : undefined;
          let payload: string | undefined;
          if (task.kind === 'death' && death) {
            payload = buildDeathPayload({
              report,
              session,
              sent,
              attempt: task.attempt ?? 1,
              cause: death.cause,
              landmark: death.landmark,
            }).text;
          }
          const r = await refineViaQwen({
            report,
            playbook,
            policy: champion,
            history: runHistory,
            reason: task.reason,
            deathPayload: payload,
            lastDiagnosis: task.sig ? (diagBySig.get(task.sig) ?? []) : [],
            prevPatch: task.sig ? patchBySig.get(task.sig) : undefined,
            lastInsights: insights
              .split('\n')
              .filter((l) => l.trim() !== '')
              .slice(-3),
            validDeathKeys: [task.key],
          });
          // 成功才占幂等键(docs/15 R6)
          evo = markAnalyzed(evo, {
            key: task.key,
            kind: task.kind,
            attempt: task.attempt,
            sig: task.sig ?? null,
            reason: task.reason,
            priority: task.priority,
          });
          evo = bumpQwen(evo, task.kind === 'death' ? 'deathAnalyses' : 'runAnalyses');
          breaker.ok();
          lastAnalysisMs = Date.now() - tA;
          analyzingKey = null;
          audit({
            v: 1,
            ts: new Date().toISOString(),
            iter,
            runId,
            actor: 'qwen',
            action: 'analyze',
            phase: 'ok',
            key: task.key,
            reason: task.reason,
            latencyMs: lastAnalysisMs,
          });
          playbook = r.playbook;
          persistPlaybook();
          if (r.insight !== null) {
            insights = appendInsight(insights, iter, insightToText(r.insight), obs);
            persistInsights();
            pushLog('text-violet-400', `经验+1:${r.insight.claim.slice(0, 24)}`, 'evolve');
          } else {
            evo = bumpQwen(evo, 'emptyInsight');
            if (r.insightIssue !== '') issues.push(r.insightIssue);
          }
          if (r.issues.some((x) => x.includes('不含 JSON'))) evo = bumpQwen(evo, 'badJson');
          if (task.sig && r.diagnosis.length > 0) {
            diagBySig.set(task.sig, [...(diagBySig.get(task.sig) ?? []), ...r.diagnosis]);
          }
          // 补丁累积(run 级合并):同 run 多次分析 → 一个候选
          const limited = limitPatch(r.policy, champion);
          const cooled = applyCooldowns(limited.policy, champion, evo.cooldowns, iter);
          Object.assign(accPatch, diffPatch(champion, cooled.policy));
          issues.push(...r.issues, ...limited.issues);
          if (task.sig && Object.keys(accPatch).length > 0) {
            accSigs.add(task.sig);
            patchBySig.set(task.sig, { ...accPatch });
          }
        } catch (e) {
          analyzingKey = null;
          const msg = e instanceof Error ? e.message : String(e);
          const isTimeout = /timeout|abort/i.test(msg);
          evo = bumpQwen(evo, isTimeout ? 'timeouts' : 'failed');
          breaker.fail();
          allOk = false;
          const retry = taskFailed(task);
          audit({
            v: 1,
            ts: new Date().toISOString(),
            iter,
            runId,
            actor: 'qwen',
            action: 'analyze',
            phase: 'fail',
            key: task.key,
            reason: task.reason,
            errClass: isTimeout ? 'timeout' : 'error',
            latencyMs: Date.now() - tA,
          });
          if (retry !== null) {
            evo = bumpQwen(evo, 'retries');
            taskQueue = [...taskQueue, retry];
            pushLog('text-amber-400', `分析失败,重试 ${retry.attempts}/2`, 'error');
          } else {
            pushLog('text-red-400', `分析失败 ${msg.slice(0, 30)}`, 'error');
          }
        }
      }
      // 局末簿记(一次/局):计数器 + 签名统计 + 迭代号
      evo = recordRun(evo, report, runId, hadTasks && allOk, mode);
      // run 级合并提案:三态干预(docs/15 §3.2)
      if (Object.keys(accPatch).length > 0 && evolveMode !== 'pause') {
        const patchPolicy = {
          ...champion,
          ...Object.fromEntries(Object.entries(accPatch).map(([k, v]) => [k, v.to])),
        } as PolicyProfile;
        const limited = limitPatch(patchPolicy, champion);
        const cooled = applyCooldowns(limited.policy, champion, evo.cooldowns, iter);
        const patch = diffPatch(champion, cooled.policy);
        issues.push(...limited.issues);
        if (Object.keys(patch).length > 0) {
          evo = bumpQwen(evo, 'patches');
          evo = noteProposals(evo, [...accSigs], iter);
          const kind = limited.kind === 'design' ? 'design' : 'numeric';
          pendingCandidate = {
            policy: cooled.policy,
            patch,
            issues,
            reason: '死亡级分析合并提案',
            kind,
          };
          persistCandidate();
          audit({
            v: 1,
            ts: new Date().toISOString(),
            iter,
            runId,
            actor: 'qwen',
            action: 'propose',
            reason: '死亡级分析合并提案',
            patch,
            patchKind: kind,
            obs,
            issues,
          });
          if (evolveMode === 'step') {
            stepGated = true;
            pushLog('text-sky-400', `提案${Object.keys(patch).length}项,单步挂起`, 'evolve');
          } else {
            armTrial();
            pushLog(
              'text-sky-400',
              `提案${Object.keys(patch).length}项:${Object.keys(patch).join(',')}`,
              'evolve',
            );
          }
        } else {
          pushLog('text-slate-400', '分析完成 · 无补丁', 'evolve');
        }
      }
      persistEvo();
    } finally {
      queueDepth = taskQueue.length + taskBacklog.length;
      qwenRefining = false;
    }
    void allOk;
  }
  async function restart(): Promise<void> {
    if (analysisGate !== null) {
      pushLog('text-sky-400', '等待 Qwen 分析完成…', 'evolve');
      await analysisGate;
    }
    phase = 'idle';
    s = createGameState(world);
    deaths = [];
    await start();
  }
  function setMode(m: DriveMode): void {
    mode = m;
    applyDriverConfig();
    keys.clear();
    pushLog('text-sky-400', `模式 → ${MODES.find((x) => x.id === m)?.label ?? m}`);
  }

  onMount(() => {
    level = buildLevelCanvas(world);
    ctx = canvasEl?.getContext('2d') ?? null;
    // 统一数据根加载(docs/13 §5):信封策略 / 手册 / 历史 / 簿记 / 审计尾;
    // 旧 localStorage 键作为一次性回退(纯 web dev 场景)。
    void (async () => {
      const store = getMarioStore();
      const { envelope } = parsePolicyFile(await store.read(POLICY_FILE));
      setPolicy(envelope.policy);
      syncSettingsFromPolicy(envelope.policy);
      playbook =
        (await store.read(PLAYBOOK_FILE)) ?? localStorage.getItem('qx-mario-playbook') ?? '';
      insights = (await store.read(INSIGHTS_FILE)) ?? '';
      runHistory = parseHistory(await store.read(HISTORY_FILE));
      if (runHistory.length === 0) {
        try {
          runHistory = parseHistory(localStorage.getItem('qx-mario-history'));
        } catch {
          /* 隐私模式 */
        }
      }
      const parsed = parseEvoState(await store.read(STATE_FILE));
      evo = parsed.state;
      auditTail = parseAuditLines(await store.read(EVOLUTION_FILE))
        .slice(-10)
        .reverse();
      // 候选恢复(docs/15 §9 教训②):重启不丢提案
      try {
        const raw = await store.read(CANDIDATE_FILE);
        if (raw !== null) {
          const c = JSON.parse(raw) as { pending: typeof pendingCandidate; trialArmed: boolean };
          pendingCandidate = c.pending;
          trialArmed = c.trialArmed === true && c.pending !== null;
        }
      } catch {
        /* 坏文件当无候选 */
      }
    })();
    void driver.probe().then(() => {
      layaOk = driver.layaReady;
      qwenOk = driver.qwenReady;
    });
    const probeTimer = setInterval(() => {
      void driver.probe().then(() => {
        layaOk = driver.layaReady;
        qwenOk = driver.qwenReady;
      });
    }, 10_000);
    const flushTimer = setInterval(() => void driver.flush(), 5_000);
    last = performance.now();
    raf = requestAnimationFrame(frame);
    return () => {
      cancelAnimationFrame(raf);
      clearInterval(probeTimer);
      clearInterval(flushTimer);
      void driver.flush();
    };
  });

  const gateBadge: Record<string, string> = {
    EXECUTE: 'text-emerald-400 bg-emerald-500/15',
    RE_SENSE: 'text-amber-400 bg-amber-500/15',
    ESCALATE: 'text-red-400 bg-red-500/15',
  };
  const dot = (ok: boolean): string => (ok ? 'bg-emerald-400' : 'bg-slate-600');
  const pctOf = (x: number): string => `${Math.min(100, (x / (COLS * TILE)) * 100).toFixed(2)}%`;
</script>

<svelte:window onkeydown={onKeyDown} onkeyup={onKeyUp} />

<div class="flex h-full min-h-0 flex-col bg-[#0b0b10] text-slate-200">
  <!-- 顶栏 40px -->
  <div class="flex h-10 flex-none items-center gap-3 border-b border-line bg-surface px-3">
    <button
      class="rounded-lg p-1.5 text-muted hover:bg-accent-soft/50 hover:text-white"
      onclick={onBack}
      aria-label="返回游戏列表"
    >
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"><path d="M19 12H5" /><path d="m12 19-7-7 7-7" /></svg
      >
    </button>
    <div class="text-[13px] font-semibold">
      Laya Jump · 1-1
      <span class="ml-2 hidden text-[11px] font-normal text-muted lg:inline"
        >Laya × Qwen 自进化</span
      >
    </div>
    <div class="ml-auto flex items-center gap-0.5 rounded-lg border border-line bg-[#0d0f14] p-0.5">
      {#each MODES as m (m.id)}
        <button
          class="rounded-md px-3 py-1 text-xs transition-colors {mode === m.id
            ? 'bg-emerald-500/15 text-emerald-400 shadow-[inset_0_0_0_1px_rgba(52,211,153,.35)]'
            : 'text-muted hover:text-white'}"
          onclick={() => setMode(m.id)}
          data-testid="mode-{m.id}"
        >
          {m.label}
        </button>
      {/each}
    </div>
    <div class="flex items-center gap-1">
      <button
        class="rounded-lg bg-emerald-600 p-1.5 text-white transition-colors hover:bg-emerald-500 disabled:opacity-40"
        onclick={start}
        disabled={!canStart || analysisPending}
        title={analysisPending ? 'Qwen 分析中,完成后可开始' : '开始'}
        data-testid="btn-start"
        aria-label={analysisPending ? '分析中' : '开始'}
      >
        {#if analysisPending}
          <svg
            class="animate-spin"
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.4"
            stroke-linecap="round"><path d="M21 12a9 9 0 1 1-6.2-8.6" /></svg
          >
        {:else}
          <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"
            ><path d="M8 5v14l11-7z" /></svg
          >
        {/if}
      </button>
      <button
        class="rounded-lg p-1.5 text-muted hover:bg-accent-soft/50 hover:text-white disabled:opacity-40"
        onclick={() => {
          if (phase === 'running') phase = 'paused';
          else if (phase === 'paused') phase = 'running';
        }}
        disabled={phase === 'idle'}
        aria-label="暂停/继续"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"
          ><rect x="6" y="5" width="4" height="14" rx="1" /><rect
            x="14"
            y="5"
            width="4"
            height="14"
            rx="1"
          /></svg
        >
      </button>
      <button
        class="rounded-lg p-1.5 text-muted hover:bg-accent-soft/50 hover:text-white"
        onclick={restart}
        aria-label="重开"
      >
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.8"
          stroke-linecap="round"><path d="M3 12a9 9 0 1 0 3-6.7L3 8" /><path d="M3 3v5h5" /></svg
        >
      </button>
    </div>
    <button
      class="rounded-lg p-1.5 text-muted hover:bg-accent-soft/50 hover:text-white {openPanels.includes(
        'settings',
      )
        ? 'bg-accent-soft/50 text-white'
        : ''}"
      onclick={() => togglePanel('settings')}
      aria-label="参数设置"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.6"
        stroke-linecap="round"
        ><circle cx="12" cy="12" r="3" /><path
          d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
        /></svg
      >
    </button>
    <button
      class="relative rounded-lg p-1.5 text-muted hover:bg-accent-soft/50 hover:text-white {openPanels.includes(
        'evo',
      )
        ? 'bg-accent-soft/50 text-white'
        : ''}"
      onclick={() => togglePanel('evo')}
      aria-label="进化面板"
      data-testid="btn-evo"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"><path d="M3 17l6-6 4 4 8-8" /><path d="M15 7h6v6" /></svg
      >
      {#if queueDepth > 0}
        <span
          class="absolute -right-0.5 -top-0.5 flex size-3.5 items-center justify-center rounded-full bg-fuchsia-400 text-[8px] font-bold text-black"
          title="待分析 {queueDepth}">{queueDepth}</span
        >
      {/if}
    </button>
    <div class="flex items-center gap-2 pl-1" title="Laya :10230 · Qwen :17230(只读探活)">
      <span class="size-1.5 rounded-full {dot(layaOk)}" data-testid="dot-laya"></span>
      <span class="size-1.5 rounded-full {dot(qwenOk)}" data-testid="dot-qwen"></span>
    </div>
  </div>

  <!-- 舞台(占满全部剩余高度) -->
  <div class="relative min-h-0 flex-1 bg-[#0b0b10]" bind:this={stageEl} data-testid="mario-stage">
    <canvas
      bind:this={canvasEl}
      class="block h-full w-full"
      style="image-rendering: pixelated;"
      data-testid="mario-canvas"
    ></canvas>

    <!-- HUD DOM 覆盖层(docs/15 §1):任意 DPI 锐利;canvas 只留像素游戏层 -->
    <div
      class="pointer-events-none absolute inset-x-0 top-0 flex items-start justify-between p-2 font-mono text-[11px] leading-none"
    >
      <div class="flex gap-3 text-white [text-shadow:0_1px_2px_rgba(0,0,0,.7)]">
        <span>SCORE {String(hud.score).padStart(6, '0')}</span>
        <span>COINS ×{String(hud.coins).padStart(2, '0')}</span>
        <span class="hidden sm:inline">WORLD 1-1</span>
      </div>
      <div class="flex items-center gap-2">
        {#if lastDecision}
          <span class="rounded bg-black/70 px-1.5 py-0.5 text-emerald-400"
            >IN {lastDecision.action}</span
          >
          <span class="rounded px-1.5 py-0.5 font-semibold {gateBadge[lastDecision.gate]}"
            >{lastDecision.gate} {(lastDecision.conf * 100).toFixed(0)}%</span
          >
        {/if}
        <span class="text-white [text-shadow:0_1px_2px_rgba(0,0,0,.7)]"
          >TIME {String(hud.time).padStart(3, '0')}</span
        >
      </div>
    </div>

    {#if phase === 'idle'}
      <div class="pointer-events-none absolute inset-0 flex items-center justify-center">
        <div class="rounded-xl bg-black/60 px-6 py-3 text-center text-sm backdrop-blur-sm">
          {#if needsLaya && !layaOk}
            需 laya-server · 去环境页启动
          {:else}
            ▶ 开始 · P 暂停 · Enter 重生
          {/if}
        </div>
      </div>
    {/if}
    {#if phase === 'paused'}
      <div class="pointer-events-none absolute inset-0 flex items-center justify-center">
        <div class="rounded-xl bg-black/60 px-6 py-2 text-sm backdrop-blur-sm">已暂停 · P 继续</div>
      </div>
    {/if}
    {#if phase === 'running' && s.phase === 'dead'}
      <div class="pointer-events-none absolute inset-0 flex items-center justify-center">
        <div
          class="rounded-xl border border-line bg-surface/90 px-6 py-4 text-center shadow-xl backdrop-blur"
        >
          <div class="text-sm font-semibold">
            阵亡 · {s.deaths[s.deaths.length - 1]?.cause ?? '?'}
          </div>
          <div class="mt-1 font-mono text-xs text-muted">
            col {Math.round(s.mario.x / TILE)} · attempt {s.attempts} · {hud.pct}%
          </div>
          <div class="mt-2 text-[11px] text-muted">Enter 立即重生(自动重生倒计时中)</div>
        </div>
      </div>
    {/if}
    {#if phase === 'running' && s.phase === 'won'}
      <div class="absolute inset-0 flex items-center justify-center">
        <div
          class="rounded-xl border border-line bg-surface/90 px-8 py-5 text-center shadow-xl backdrop-blur"
        >
          <div class="text-base font-semibold text-emerald-400">通关 · 1-1</div>
          <div class="mt-1 font-mono text-xs text-muted">
            score {hud.score} · coins {hud.coins} · attempt {hud.life}
          </div>
          <button
            class="mt-3 rounded-lg bg-emerald-600 px-4 py-1.5 text-xs text-white hover:bg-emerald-500"
            onclick={restart}
          >
            再来一次(⏎)
          </button>
        </div>
      </div>
    {/if}

    <!-- LAYA 决策浮层(FloatingPanel,可拖动/折叠/置顶) -->
    {#if openPanels.includes('laya')}
      <FloatingPanel
        title="Laya 决策 · 最近一拍"
        testid="panel-laya"
        width={380}
        initial={{ x: 24, y: 24 }}
        persistKey="laya"
        onclose={() => togglePanel('laya')}
      >
        {#if lastDecision}
          <div class="space-y-1.5">
            {#each lastDecision.probs.slice(0, 6) as [a, p] (a)}
              <div class="flex items-center gap-2 text-xs">
                <span class="w-[110px] truncate font-mono text-muted">{a}</span>
                <div class="h-1.5 flex-1 overflow-hidden rounded bg-[#1d2230]">
                  <div
                    class="h-full rounded bg-emerald-500"
                    style={`width:${(p * 100).toFixed(1)}%`}
                  ></div>
                </div>
                <span class="w-10 text-right font-mono">{(p * 100).toFixed(0)}%</span>
              </div>
            {/each}
          </div>
          <div class="mt-3 grid grid-cols-4 gap-2">
            <div class="rounded-lg border border-line bg-[#151823] px-2 py-1.5">
              <div class="font-mono text-sm">{(lastDecision.conf * 100).toFixed(0)}%</div>
              <div class="text-[10px] text-muted">置信度</div>
            </div>
            <div class="rounded-lg border border-line bg-[#151823] px-2 py-1.5">
              <div class="font-mono text-sm">{stats.exec}</div>
              <div class="text-[10px] text-muted">EXEC</div>
            </div>
            <div class="rounded-lg border border-line bg-[#151823] px-2 py-1.5">
              <div class="font-mono text-sm text-amber-400">{stats.reSense}</div>
              <div class="text-[10px] text-muted">RE-SENSE</div>
            </div>
            <div class="rounded-lg border border-line bg-[#151823] px-2 py-1.5">
              <div class="font-mono text-sm text-red-400">{stats.escalate}</div>
              <div class="text-[10px] text-muted">ESCALATE</div>
            </div>
          </div>
          <details class="mt-3">
            <summary class="cursor-pointer text-[10px] uppercase tracking-wider text-muted"
              >Laya 输入</summary
            >
            <pre
              class="mt-2 max-h-40 overflow-auto rounded-lg bg-[#0d0f14] p-2 font-mono text-[10px] leading-relaxed text-slate-400">{stateJson}</pre>
          </details>
        {:else}
          <div class="text-xs text-muted">尚无决策 —— 开始后生效</div>
        {/if}
        <!-- 日志(三档过滤,docs/15 §5.6;异常档恒显) -->
        <div class="mt-2 flex gap-1">
          {#each [['all', '全部'], ['game', '玩法'], ['evolve', '进化'], ['error', '异常']] as [id, label] (id)}
            <button
              class="rounded px-1.5 py-0.5 text-[10px] {logFilter === id
                ? 'bg-accent-soft/60 text-white'
                : 'text-muted hover:text-white'}"
              onclick={() => (logFilter = id as typeof logFilter)}
            >
              {label}
            </button>
          {/each}
        </div>
        <div
          class="mt-1 max-h-36 overflow-auto rounded-lg bg-[#0d0f14] p-2"
          data-testid="log-stream"
        >
          {#if log.length === 0}
            <div class="text-[10px] text-muted">日志空</div>
          {:else}
            {#each log.filter((r) => logFilter === 'all' || r.level === logFilter || r.level === 'error') as row (row.t + row.text)}
              <div class="flex gap-2 font-mono text-[10px] leading-relaxed">
                <span class="flex-none text-slate-600">{row.t}</span>
                <span class="truncate {row.cls}">{row.text}</span>
              </div>
            {/each}
          {/if}
        </div>
      </FloatingPanel>
    {/if}

    <!-- 参数设置浮层 -->
    {#if openPanels.includes('settings')}
      <FloatingPanel
        title="参数 · 即时生效"
        testid="panel-settings"
        width={340}
        initial={{ x: 48, y: 48 }}
        persistKey="settings"
        onclose={() => togglePanel('settings')}
      >
        <div class="space-y-3 text-xs">
          <label class="block">
            <div class="mb-1 flex justify-between">
              <span class="text-muted">速度</span><span class="font-mono"
                >{settings.speed.toFixed(1)}×</span
              >
            </div>
            <input
              class="w-full accent-emerald-500"
              type="range"
              min="0.5"
              max="3"
              step="0.1"
              bind:value={settings.speed}
            />
          </label>
          <label class="block">
            <div class="mb-1 flex justify-between">
              <span class="text-muted">决策间隔</span><span class="font-mono"
                >{settings.intervalMs}ms</span
              >
            </div>
            <input
              class="w-full accent-emerald-500"
              type="range"
              min="150"
              max="600"
              step="25"
              bind:value={settings.intervalMs}
              onchange={applyDriverConfig}
            />
          </label>
          <label class="block">
            <div class="mb-1 flex justify-between">
              <span class="text-muted">门控 EXECUTE</span><span class="font-mono"
                >{settings.gateExecute.toFixed(2)}</span
              >
            </div>
            <input
              class="w-full accent-emerald-500"
              type="range"
              min="0.05"
              max="0.5"
              step="0.01"
              bind:value={settings.gateExecute}
              onchange={applyDriverConfig}
            />
          </label>
          <label class="block">
            <div class="mb-1 flex justify-between">
              <span class="text-muted">门控 ESCALATE</span><span class="font-mono"
                >{settings.gateEscalate.toFixed(2)}</span
              >
            </div>
            <input
              class="w-full accent-emerald-500"
              type="range"
              min="0.02"
              max="0.3"
              step="0.01"
              bind:value={settings.gateEscalate}
              onchange={applyDriverConfig}
            />
          </label>
        </div>
      </FloatingPanel>
    {/if}

    <!-- 复盘浮层 -->
    {#if openPanels.includes('pm')}
      <FloatingPanel
        title="复盘"
        testid="panel-pm"
        width={520}
        initial={{ x: 72, y: 72 }}
        persistKey="pm"
        onclose={() => togglePanel('pm')}
      >
        {#if postmortem}
          <pre
            class="max-h-[60vh] overflow-auto rounded-lg bg-[#0d0f14] p-3 font-mono text-[10px] leading-relaxed text-slate-300">{postmortem}</pre>
        {:else}
          <div class="text-xs text-muted">本局尚无复盘 —— 通关 / 第 3 次死亡自动生成</div>
          <button
            class="mt-2 rounded-md border border-line px-2 py-1 text-[11px] text-muted hover:text-white"
            onclick={genPostmortem}
          >
            立即生成复盘
          </button>
        {/if}
      </FloatingPanel>
    {/if}

    <!-- 进化面板(docs/15 §2/§3):状态/统计/学到了什么/只读时间线 -->
    {#if openPanels.includes('evo')}
      <FloatingPanel
        title="进化"
        testid="panel-evo"
        width={480}
        initial={{ x: 96, y: 96 }}
        persistKey="evo"
        onclose={() => togglePanel('evo')}
      >
        <div class="mb-2 flex items-center justify-between">
          <span class="font-mono text-[11px]">
            #{evo.iteration} · champion
            <b class="text-white"
              >{evo.championScore === null ? '未测' : evo.championScore.toFixed(0)}</b
            >
            · 最远 {evo.bestMaxXCol} 列
          </span>
          <div class="flex gap-0.5 rounded-lg border border-line bg-[#0d0f14] p-0.5">
            {#each [['auto', '自动'], ['step', '单步'], ['pause', '暂停']] as [id, label] (id)}
              <button
                class="rounded px-2 py-0.5 text-[10px] {evolveMode === id
                  ? 'bg-emerald-500/15 text-emerald-400'
                  : 'text-muted hover:text-white'}"
                onclick={() => (evolveMode = id as typeof evolveMode)}
                data-testid="evo-mode-{id}"
              >
                {label}
              </button>
            {/each}
          </div>
        </div>

        <!-- 当前候选/消息一行摘要 -->
        <div class="mb-2 font-mono text-[10px] text-muted">
          {#if pendingCandidate}
            本轮:试用 {Object.keys(pendingCandidate.patch).join(',')}
            {#if stepGated}· 单步挂起{/if}
          {:else}
            本轮:无候选
          {/if}
          · 消息:{getPolicy().obsThreatFormat}/{getPolicy().obsProgressStyle}/{getPolicy()
            .obsHintStyle}/
          {getPolicy().obsInstructionVariant}
        </div>

        <!-- 统计(docs/15 §2) -->
        <div class="mb-2 grid grid-cols-4 gap-1.5 text-center">
          <div class="rounded-lg border border-line bg-[#151823] px-1 py-1.5">
            <div class="font-mono text-sm">{evo.counters.deaths}</div>
            <div class="text-[10px] text-muted">失败</div>
          </div>
          <div class="rounded-lg border border-line bg-[#151823] px-1 py-1.5">
            <div class="font-mono text-sm">
              {evo.counters.wins}/{evo.counters.runs}
            </div>
            <div class="text-[10px] text-muted">通关</div>
          </div>
          <div class="rounded-lg border border-line bg-[#151823] px-1 py-1.5">
            <div class="font-mono text-sm">
              {evo.counters.qwen.deathAnalyses + evo.counters.qwen.runAnalyses}
              {#if evo.counters.qwen.emptyInsight > 0}<span class="text-amber-400"
                  >(空{evo.counters.qwen.emptyInsight})</span
                >{/if}
            </div>
            <div class="text-[10px] text-muted">分析</div>
          </div>
          <div class="rounded-lg border border-line bg-[#151823] px-1 py-1.5">
            <div class="font-mono text-sm">
              {insights.split('\n').filter((l) => l.startsWith('- ')).length}
            </div>
            <div class="text-[10px] text-muted">经验</div>
          </div>
        </div>
        <div class="mb-2 font-mono text-[10px] text-muted">
          漏斗 提案{evo.counters.qwen.patches} → 转正{evo.counters.qwen.commits} → 回滚{evo.counters
            .qwen.rollbacks}
          · 队列 {queueDepth}{#if analyzingKey}
            · 析 {analyzingKey}{/if}{#if lastAnalysisMs !== null}
            · 上次 {(lastAnalysisMs / 1000).toFixed(1)}s{/if}
          {#if Object.keys(evo.cooldowns).some((k) => (evo.cooldowns[k] ?? 0) > evo.iteration)}
            · 冷却 {Object.entries(evo.cooldowns)
              .filter(([, until]) => until > evo.iteration)
              .map(([k, until]) => `${k}→${until}`)
              .join(' ')}
          {/if}
        </div>

        <!-- 单步放行 -->
        {#if stepGated}
          <button
            class="mb-2 w-full rounded-md bg-sky-600 px-2 py-1 text-[11px] text-white hover:bg-sky-500"
            onclick={stepAdvance}
            data-testid="btn-step-advance"
          >
            下一步 ▸(装填候选)
          </button>
        {/if}

        <!-- 学到了什么:最近 3 条经验 -->
        {#if insights}
          <div class="mb-2">
            <div class="text-[10px] uppercase tracking-[0.08em] text-muted">学到了什么</div>
            {#each insights
              .split('\n')
              .filter((l) => l.startsWith('- '))
              .slice(-3)
              .reverse() as line (line)}
              <div class="truncate font-mono text-[10px] text-violet-200/80" title={line}>
                ✦ {line.replace(/^- \[iter \d+\] /, '')}
              </div>
            {/each}
          </div>
        {/if}

        <!-- 手册折叠 -->
        {#if playbook}
          <details class="mb-2">
            <summary class="cursor-pointer text-[11px] text-muted hover:text-white">
              策略手册({playbook.length} 字)
            </summary>
            <pre
              class="mt-1 max-h-[20vh] overflow-auto rounded-lg bg-[#0d0f14] p-2 font-mono text-[10px] leading-relaxed text-slate-300">{playbook}</pre>
          </details>
        {/if}

        <!-- 时间线(只读) -->
        <div class="text-[10px] uppercase tracking-[0.08em] text-muted">时间线</div>
        {#if auditTail.length > 0}
          <div class="mt-1 max-h-[18vh] overflow-auto font-mono text-[10px] leading-relaxed">
            {#each auditTail as a (a.ts + a.action + a.key)}
              <div class="text-slate-400">
                <span class="text-muted">#{a.iter}</span>
                <span
                  class={a.action === 'commit' || a.phase === 'ok'
                    ? 'text-emerald-400'
                    : a.action === 'rollback' || a.phase === 'fail'
                      ? 'text-red-400'
                      : a.action === 'propose'
                        ? 'text-fuchsia-300'
                        : 'text-muted'}>{a.actor === 'qwen' ? 'Q' : '⚙'}</span
                >
                {a.action === 'analyze'
                  ? `${a.phase === 'fail' ? '析失败' : '析'} ${(a.key ?? '').split('#').slice(-1)[0] ?? ''} ${reasonShort(a.reason)}${a.latencyMs ? ` ${(a.latencyMs / 1000).toFixed(1)}s` : ''}`
                  : `${a.action === 'propose' ? `提案 ${Object.keys(a.patch ?? {}).join(',')}` : reasonShort(a.reason)}`}
              </div>
            {/each}
          </div>
        {:else}
          <div class="mt-1 text-[11px] text-muted">尚无记录</div>
        {/if}
        <button
          class="mt-2 rounded-md border border-line px-2 py-0.5 text-[10px] text-muted hover:text-white"
          onclick={resetPolicy}
          data-testid="btn-reset-policy"
        >
          重置策略
        </button>
      </FloatingPanel>
    {/if}
  </div>
  <div class="relative h-[3px] flex-none bg-[#171a22]" data-testid="progress-strip">
    {#each deaths as d, i (`${d.x}-${i}`)}
      <div
        class="absolute top-0 h-[3px] w-[2px] bg-red-400/80"
        style={`left:${pctOf(d.x)}`}
        title={`死亡 ${d.cause}`}
      ></div>
    {/each}
    <div
      class="absolute -top-[2px] size-[7px] rounded-full bg-emerald-400 shadow-[0_0_6px_rgba(52,211,153,.8)]"
      style={`left:calc(${hud.pct}% - 3px)`}
      data-testid="mario-dot"
    ></div>
    <div
      class="absolute -top-[2px] h-[7px] w-[3px] rounded-sm bg-fuchsia-400/80"
      style={`left:${pctOf(world.data.flagCol * TILE)}`}
    ></div>
  </div>

  <!-- 遥测条:单行摘要,详情进浮层 -->
  <div
    class="flex h-[52px] flex-none flex-col justify-center gap-1 border-t border-line bg-surface px-3"
  >
    <div class="flex items-center gap-2.5 text-xs">
      <span
        class="flex size-[18px] flex-none items-center justify-center rounded-md bg-emerald-500/10 font-mono text-[11px] text-emerald-400"
        >L</span
      >
      <span class="w-8 flex-none text-[10px] uppercase tracking-[0.08em] text-muted">laya</span>
      {#if needsLaya && !layaOk}
        <span class="truncate text-red-400" data-testid="telem-l"
          >未就绪 —— 请在 release 版 · 环境页启动 laya-server(:10230)</span
        >
      {:else if lastDecision}
        <span class="truncate font-mono" data-testid="telem-l">
          {lastDecision.action} · <b class="text-white">{(lastDecision.conf * 100).toFixed(0)}%</b>
          ·
          <span
            class="rounded px-1.5 py-px text-[10px] font-semibold {gateBadge[lastDecision.gate]}"
            >{lastDecision.gate}</span
          >
          · {lastDecision.latencyMs.toFixed(0)}ms · {stats.qps}Hz · x {hud.pct}%
        </span>
      {:else}
        <span class="truncate text-muted" data-testid="telem-l">
          待机 —— {mode === 'human'
            ? '人玩模式不跑决策'
            : mode === 'autopilot'
              ? '自驾 = 规划器直控(不经 Laya)'
              : `开始后每 ${settings.intervalMs}ms 一拍`}
        </span>
      {/if}
      <button
        class="flex-none rounded-md border border-line px-2 py-0.5 text-[11px] {postmortem
          ? 'border-sky-500/40 text-sky-400'
          : 'text-muted hover:text-white'}"
        onclick={() => {
          if (!postmortem && (mode === 'reflex' || mode === 'pure')) genPostmortem();
          togglePanel('pm');
        }}
        data-testid="btn-pm"
      >
        复盘 {postmortem ? '●' : '▸'}
      </button>
      <button
        class="ml-auto flex-none rounded-md border border-line px-2 py-0.5 text-[11px] text-muted hover:text-white"
        onclick={() => togglePanel('laya')}
      >
        详情 ▸
      </button>
    </div>
    <div class="flex items-center gap-2.5 text-xs">
      <span
        class="flex size-[18px] flex-none items-center justify-center rounded-md bg-[#1d2230] font-mono text-[11px] text-slate-400"
        >Σ</span
      >
      <span class="w-8 flex-none text-[10px] uppercase tracking-[0.08em] text-muted">统计</span>
      <span class="truncate font-mono text-muted" data-testid="telem-stats">
        决策 <b class="text-slate-200">{stats.decisions}</b> · EXEC {stats.exec} · RE {stats.reSense}
        · ESC {stats.escalate} · stale {stats.stale} · veto
        <b class="text-orange-300">{stats.vetoes}</b> · conf {stats.avgConf.toFixed(2)} · {stats.avgLatencyMs.toFixed(
          0,
        )}ms · coins {hud.coins} · 命 {hud.life} · time {hud.time}
      </span>
      <span class="ml-auto flex-none text-[10px] text-muted">
        {mode === 'autopilot'
          ? '规划器直控'
          : mode === 'human'
            ? '←→ 移动 · Z/空格 跳 · Shift 跑'
            : `Laya ${layaOk ? ':10230 ✓' : '✗'}`}
      </span>
    </div>
  </div>
</div>
