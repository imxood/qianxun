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
  import { SvelteSet } from 'svelte/reactivity';

  import type { PostmortemReport } from './postmortem';
  import { getPolicy, setPolicy, type PolicyProfile } from './policy';
  import { refineViaQwen, type HistoryRow } from './qwen';
  import {
    EVOLUTION_FILE,
    HISTORY_FILE,
    INSIGHTS_FILE,
    PLAYBOOK_FILE,
    POLICY_FILE,
    STATE_FILE,
    applyCooldowns,
    appendHistory,
    appendInsight,
    auditLine,
    diffPatch,
    initialEvoState,
    judgeCandidate,
    limitPatch,
    obsSnapshotOf,
    parseAuditLines,
    parseEvoState,
    parseHistory,
    parsePolicyFile,
    recordRun,
    runScore,
    shouldInvokeQwen,
    startCooldowns,
    wrapPolicy,
    type AuditEntry,
    type EvoState,
  } from './evolution';
  import { getMarioStore } from './store';

  import { MarioDriver, type LogRow } from './driver';
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
  let log = $state<LogRow[]>([]);
  let deaths = $state<Array<{ x: number; cause: string }>>([]);
  let panel = $state<'laya' | 'settings' | 'pm' | 'evo' | null>(null);
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

  // ---- 进化闭环(docs/13):champion 持久于数据根;Qwen 补丁是候选,试用不热生效 ----
  let evo = $state<EvoState>(initialEvoState());
  let championSource = $state('default');
  /** 本局策略快照(局内不可变,docs/13 不变量①);null = 尚未开局。 */
  let runPolicy: PolicyProfile | null = null;
  /** 待决候选:Qwen 提案,经 下局试用 / 采纳 / 丢弃 处置。 */
  let pendingCandidate = $state<{
    policy: PolicyProfile;
    patch: Record<string, { from: unknown; to: unknown }>;
    issues: string[];
    reason: string;
  } | null>(null);
  /** 用户点了「下局试用」的候选;开局时转为 trialCandidate。 */
  let trialArmed = $state(false);
  /** 本局正在试用的候选(局后单局计分判定)。 */
  let trialCandidate: {
    policy: PolicyProfile;
    patch: Record<string, { from: unknown; to: unknown }>;
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

  function pushLog(cls: string, text: string): void {
    const d = new Date();
    const t = `${d.toLocaleTimeString('zh-CN', { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`;
    log = [{ t, cls, text }, ...log].slice(0, 40);
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

  // ---- 候选处置(docs/13 §4:LLM 负责创意,代码负责纪律) ----
  function armTrial(): void {
    trialArmed = true;
    pushLog('text-sky-400', '候选已装填:下一局试用(局内快照,结束后按 champion 基线判定)');
  }
  function adoptCandidate(): void {
    const c = pendingCandidate;
    if (!c) return;
    setPolicy(c.policy);
    syncSettingsFromPolicy(c.policy);
    persistPolicy(`commit:user@iter-${evo.iteration}`);
    audit({
      v: 1,
      ts: new Date().toISOString(),
      iter: evo.iteration,
      runId: null,
      actor: 'user',
      action: 'commit',
      reason: `人工采纳(${c.reason})`,
      patch: c.patch,
      issues: c.issues,
    });
    pendingCandidate = null;
    trialArmed = false;
    pushLog('text-emerald-400', '候选已人工采纳为 champion(下一局生效)');
  }
  function discardCandidate(): void {
    const c = pendingCandidate;
    if (!c) return;
    evo = { ...evo, cooldowns: startCooldowns(evo.cooldowns, Object.keys(c.patch), evo.iteration) };
    persistEvo();
    audit({
      v: 1,
      ts: new Date().toISOString(),
      iter: evo.iteration,
      runId: null,
      actor: 'user',
      action: 'skip',
      reason: `人工丢弃(${c.reason})`,
      patch: c.patch,
      issues: c.issues,
    });
    pendingCandidate = null;
    trialArmed = false;
    pushLog('text-amber-400', '候选已丢弃,字段进入冷却期(5 局内 Qwen 不再重复提案)');
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
    pushLog(
      d.gate === 'EXECUTE'
        ? 'text-emerald-400'
        : d.gate === 'RE_SENSE'
          ? 'text-amber-400'
          : 'text-red-400',
      `${d.gate} ${d.action} conf=${d.conf.toFixed(2)} (${d.latencyMs.toFixed(0)}ms) ${d.note}`,
    );
  };
  driver.onLog = (row: LogRow) => pushLog(row.cls, row.text);

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
    if (down && e.code === 'Escape' && panel) {
      panel = null;
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
              `死亡 ${e.cause} @ col ${Math.round(s.mario.x / TILE)}(attempt ${s.attempts})`,
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
            pushLog('text-emerald-400', `通关!加分 ${e.bonus} · score ${s.score}`);
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
  function start(): void {
    if (phase === 'running' || !canStart) return;
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
      runPolicy = { ...pendingCandidate.policy };
      pushLog('text-sky-400', `本局试用候选:${Object.keys(trialCandidate.patch).join(', ')}`);
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
    pushLog('text-sky-400', '已生成复盘文档(详情浮层可看;已推入会话 JSONL)');
    runHistory = appendHistory(runHistory, {
      iteration: runHistory.length + 1,
      won: s.phase === 'won',
      maxXCol: report.maxXCol,
      ticks: s.tick,
    });
    persistHistory();

    // ① 试用候选的局后判定:单局计分 vs champion 基线(+ε 滞回)
    if (trialCandidate !== null) {
      const c = trialCandidate;
      trialCandidate = null;
      const score = runScore({ won: s.phase === 'won', maxXCol: report.maxXCol, ticks: s.tick });
      if (evo.championScore !== null) {
        const v = judgeCandidate(evo.championScore, score);
        if (v.verdict === 'commit') {
          setPolicy(c.policy);
          syncSettingsFromPolicy(c.policy);
          persistPolicy(`commit:trial@iter-${evo.iteration}`);
          evo = { ...evo, championScore: score };
          pendingCandidate = null;
          pushLog('text-emerald-400', `试用候选转正:${v.reason}(得分 ${score.toFixed(0)})`);
        } else {
          evo = {
            ...evo,
            cooldowns: startCooldowns(evo.cooldowns, Object.keys(c.patch), evo.iteration),
          };
          pendingCandidate = null;
          pushLog('text-amber-400', `试用候选回滚:${v.reason}(得分 ${score.toFixed(0)})`);
        }
        audit({
          v: 1,
          ts: new Date().toISOString(),
          iter: evo.iteration,
          runId: driver.runId,
          actor: 'system',
          action: v.verdict,
          reason: `UI 单局试用 · ${v.reason}`,
          patch: c.patch,
          score: { champion: evo.championScore, candidate: score },
        });
      } else {
        pushLog(
          'text-sky-400',
          `试用得分 ${score.toFixed(0)}(champion 无基线,请在进化面板人工 采纳/丢弃)`,
        );
      }
    }

    // ② Qwen 触发器(docs/14 §2):每死必析,签名重复只标注不去重;runId 幂等
    const trigger = shouldInvokeQwen(evo, report, driver.runId);
    if (qwenOk && trigger.invoke) {
      evo = recordRun(evo, report, driver.runId, true);
      void qwenRefineAfterRun(report, trigger.reason);
    } else {
      evo = recordRun(evo, report, driver.runId, false);
      if (qwenOk) pushLog('text-slate-500', `局后不调 Qwen:${trigger.reason}`);
    }
    persistEvo();
  }

  /**
   * 局后分析(docs/14):每局死亡/通关都调用本地 Qwen(:17230)。
   * ①新版手册(≤1600 字)立即沉淀持久;②策略补丁经 sanitize → 限幅 → 冷却
   * 过滤后成为**候选**,绝不热生效——由用户 试用/采纳/丢弃 处置;
   * ③insight(可泛化 Laya 经验,≤200 字)追加进 insights.md。
   * 失败静默降级——规则复盘已在手,游戏照常。
   */
  async function qwenRefineAfterRun(report: PostmortemReport, reason: string): Promise<void> {
    if (qwenRefining) return;
    qwenRefining = true;
    try {
      const champion = getPolicy();
      const obs = obsSnapshotOf(champion);
      const r = await refineViaQwen({
        report,
        playbook,
        policy: champion,
        history: runHistory,
        reason,
      });
      playbook = r.playbook;
      persistPlaybook();
      if (r.insight !== '') {
        insights = appendInsight(insights, evo.iteration, r.insight, obs);
        persistInsights();
        pushLog(
          'text-violet-400',
          `Laya 经验 +1:${r.insight.slice(0, 60)}${r.insight.length > 60 ? '…' : ''}`,
        );
      }
      const limited = limitPatch(r.policy, champion);
      const cooled = applyCooldowns(limited.policy, champion, evo.cooldowns, evo.iteration);
      const patch = diffPatch(champion, cooled.policy);
      const issues = [...r.issues, ...limited.issues];
      if (Object.keys(patch).length === 0) {
        audit({
          v: 1,
          ts: new Date().toISOString(),
          iter: evo.iteration,
          runId: driver.runId,
          actor: 'qwen',
          action: 'skip',
          reason: '补丁为空或与 champion 无差异',
          obs,
          issues,
        });
        pushLog('text-slate-400', `Qwen 分析完成:手册 ${playbook.length} 字 · 补丁无有效变更`);
        return;
      }
      pendingCandidate = { policy: cooled.policy, patch, issues, reason };
      audit({
        v: 1,
        ts: new Date().toISOString(),
        iter: evo.iteration,
        runId: driver.runId,
        actor: 'qwen',
        action: 'propose',
        reason,
        patch,
        obs,
        issues,
      });
      pushLog(
        'text-sky-400',
        `Qwen 提案 ${Object.keys(patch).length} 字段 → 候选(进化面板处置) · 手册 ${playbook.length} 字` +
          (issues.length > 0 ? ` · 纪律钳制 ${issues.length} 项` : ''),
      );
    } catch (e) {
      pushLog(
        'text-amber-400',
        `Qwen 局后分析失败(不影响游戏):${e instanceof Error ? e.message : String(e)}`,
      );
    } finally {
      qwenRefining = false;
    }
  }
  function restart(): void {
    phase = 'idle';
    s = createGameState(world);
    deaths = [];
    start();
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
      championSource = envelope.source;
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
        >System 1 反射 × 规划器 × (Qwen 监督 · M6)</span
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
        disabled={!canStart}
        data-testid="btn-start"
        aria-label="开始"
      >
        <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"
          ><path d="M8 5v14l11-7z" /></svg
        >
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
      class="rounded-lg p-1.5 text-muted hover:bg-accent-soft/50 hover:text-white {panel ===
      'settings'
        ? 'bg-accent-soft/50 text-white'
        : ''}"
      onclick={() => (panel = panel === 'settings' ? null : 'settings')}
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
      class="relative rounded-lg p-1.5 text-muted hover:bg-accent-soft/50 hover:text-white {panel ===
      'evo'
        ? 'bg-accent-soft/50 text-white'
        : ''}"
      onclick={() => (panel = panel === 'evo' ? null : 'evo')}
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
      {#if pendingCandidate}
        <span
          class="absolute -right-0.5 -top-0.5 size-2 rounded-full bg-fuchsia-400"
          title="有待决候选"
        ></span>
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

    {#if phase === 'idle'}
      <div class="pointer-events-none absolute inset-0 flex items-center justify-center">
        <div class="rounded-xl bg-black/60 px-6 py-3 text-center text-sm backdrop-blur-sm">
          {#if needsLaya && !layaOk}
            反射/纯反射需要 laya-server —— 请在 release 版 · 环境页启动<br />
            <span class="text-[11px] text-muted">(游戏页只读探活,绝不代启 · docs/11 §1.1)</span>
          {:else}
            选择模式,按 ▶ 开始(P 暂停 / Enter 死后立刻重生)
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

    <!-- LAYA 决策浮层 -->
    {#if panel === 'laya'}
      <div
        class="absolute bottom-3 right-3 w-[380px] rounded-xl border border-line bg-surface/95 p-4 shadow-2xl backdrop-blur"
        data-testid="panel-laya"
      >
        <div class="mb-3 flex items-center justify-between">
          <span class="text-[10px] uppercase tracking-[0.08em] text-muted"
            >Laya 决策 · 最近一拍</span
          >
          <button class="text-xs text-muted hover:text-white" onclick={() => (panel = null)}
            >✕</button
          >
        </div>
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
              >state 预览(Laya 眼中)</summary
            >
            <pre
              class="mt-2 max-h-40 overflow-auto rounded-lg bg-[#0d0f14] p-2 font-mono text-[10px] leading-relaxed text-slate-400">{stateJson}</pre>
          </details>
        {:else}
          <div class="text-xs text-muted">尚无决策 —— 开始后生效</div>
        {/if}
        <!-- 决策日志流(含 laya-server 不可达等错误行,最新在上) -->
        <div
          class="mt-3 max-h-36 overflow-auto rounded-lg bg-[#0d0f14] p-2"
          data-testid="log-stream"
        >
          {#if log.length === 0}
            <div class="text-[10px] text-muted">日志空 —— 决策/事件/错误都会落在这里</div>
          {:else}
            {#each log as row (row.t + row.text)}
              <div class="flex gap-2 font-mono text-[10px] leading-relaxed">
                <span class="flex-none text-slate-600">{row.t}</span>
                <span class="truncate {row.cls}">{row.text}</span>
              </div>
            {/each}
          {/if}
        </div>
      </div>
    {/if}

    <!-- 参数设置浮层 -->
    {#if panel === 'settings'}
      <div
        class="absolute bottom-3 right-3 w-[340px] rounded-xl border border-line bg-surface/95 p-4 shadow-2xl backdrop-blur"
      >
        <div class="mb-3 flex items-center justify-between">
          <span class="text-[10px] uppercase tracking-[0.08em] text-muted">参数 · 即时生效</span>
          <button class="text-xs text-muted hover:text-white" onclick={() => (panel = null)}
            >✕</button
          >
        </div>
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
      </div>
    {/if}

    <!-- 复盘浮层:规则判定的总结文档(本地 Qwen / 云端编码代理消费同一格式) -->
    {#if panel === 'pm'}
      <div
        class="absolute bottom-3 right-3 w-[520px] rounded-xl border border-line bg-surface/95 p-4 shadow-2xl backdrop-blur"
        data-testid="panel-pm"
      >
        <div class="mb-3 flex items-center justify-between">
          <span class="text-[10px] uppercase tracking-[0.08em] text-muted">
            复盘 · 规则判定(无 LLM) · 消费方:本地 Qwen / 编码代理
          </span>
          <button class="text-xs text-muted hover:text-white" onclick={() => (panel = null)}
            >✕</button
          >
        </div>
        {#if postmortem}
          <pre
            class="max-h-[60vh] overflow-auto rounded-lg bg-[#0d0f14] p-3 font-mono text-[10px] leading-relaxed text-slate-300">{postmortem}</pre>
          <div class="mt-2 text-[10px] text-muted">
            批量版(每局 summary.md + 跨局 INDEX.md):
            <code>npx tsx e2e/mario-postmortem.ts [sessionsDir]</code>
          </div>
        {:else}
          <div class="text-xs text-muted">
            本局尚无复盘 —— 通关 / 第 3 次死亡自动生成;或点下方立即生成
          </div>
          <button
            class="mt-2 rounded-md border border-line px-2 py-1 text-[11px] text-muted hover:text-white"
            onclick={genPostmortem}
          >
            立即生成复盘
          </button>
        {/if}
      </div>
    {/if}

    <!-- 进化面板(docs/13):champion 摘要 / 手册 / 候选处置 / 冷却期 / 审计尾 -->
    {#if panel === 'evo'}
      <div
        class="absolute bottom-3 right-3 w-[480px] rounded-xl border border-line bg-surface/95 p-4 shadow-2xl backdrop-blur"
        data-testid="panel-evo"
      >
        <div class="mb-3 flex items-center justify-between">
          <span class="text-[10px] uppercase tracking-[0.08em] text-muted">
            进化闭环 · 候选-评估-提交 · 数据根 mario/
          </span>
          <button class="text-xs text-muted hover:text-white" onclick={() => (panel = null)}
            >✕</button
          >
        </div>

        <!-- champion 摘要 -->
        <div class="mb-3 rounded-lg bg-[#0d0f14] p-3 font-mono text-[11px] leading-relaxed">
          <div>
            champion · <span class="text-sky-400">{championSource}</span> · 基线分
            <b class="text-white"
              >{evo.championScore === null ? '未测' : evo.championScore.toFixed(0)}</b
            >
          </div>
          <div class="text-muted">
            迭代 {evo.iteration} · 最佳 maxX {evo.bestMaxXCol}col · 平台期 {evo.plateauStreak} · 历史
            {runHistory.length} 局 · 手册 {playbook.length} 字
          </div>
        </div>

        <!-- 待决候选 -->
        {#if pendingCandidate}
          <div class="mb-3 rounded-lg border border-fuchsia-500/40 bg-fuchsia-500/5 p-3">
            <div class="mb-1.5 flex items-center justify-between">
              <span class="text-[11px] font-semibold text-fuchsia-300">
                待决候选 · {pendingCandidate.reason}
              </span>
              {#if trialArmed}
                <span class="rounded bg-sky-500/20 px-1.5 py-0.5 text-[10px] text-sky-300"
                  >已装填,下局试用</span
                >
              {/if}
            </div>
            <div class="mb-2 font-mono text-[11px] leading-relaxed text-slate-300">
              {#each Object.entries(pendingCandidate.patch) as [k, v] (k)}
                <div>
                  {k}: <span class="text-muted">{JSON.stringify(v.from)}</span> →
                  <span class="text-fuchsia-300">{JSON.stringify(v.to)}</span>
                </div>
              {/each}
              {#if pendingCandidate.issues.length > 0}
                <div class="mt-1 text-[10px] text-amber-400/80">
                  纪律:{pendingCandidate.issues.join('; ')}
                </div>
              {/if}
            </div>
            <div class="flex gap-2">
              <button
                class="rounded-md bg-sky-600 px-2.5 py-1 text-[11px] text-white hover:bg-sky-500 disabled:opacity-40"
                onclick={armTrial}
                disabled={trialArmed}
                data-testid="btn-trial">下局试用</button
              >
              <button
                class="rounded-md bg-emerald-600 px-2.5 py-1 text-[11px] text-white hover:bg-emerald-500"
                onclick={adoptCandidate}
                data-testid="btn-adopt">采纳</button
              >
              <button
                class="rounded-md border border-line px-2.5 py-1 text-[11px] text-muted hover:text-white"
                onclick={discardCandidate}
                data-testid="btn-discard">丢弃</button
              >
            </div>
            <div class="mt-1.5 text-[10px] text-muted">
              试用 = 下一局以快照跑候选,局后单局计分 vs 基线(+5% 滞回);局内绝不热生效
            </div>
          </div>
        {/if}

        <!-- 冷却期 -->
        {#if Object.keys(evo.cooldowns).some((k) => (evo.cooldowns[k] ?? 0) > evo.iteration)}
          <div class="mb-3 font-mono text-[10px] text-muted">
            冷却:{Object.entries(evo.cooldowns)
              .filter(([, until]) => until > evo.iteration)
              .map(([k, until]) => `${k}→#${until}`)
              .join(' · ')}
          </div>
        {/if}

        <!-- 手册预览 -->
        {#if playbook}
          <details class="mb-3">
            <summary class="cursor-pointer text-[11px] text-muted hover:text-white">
              策略手册预览({playbook.length} 字)
            </summary>
            <pre
              class="mt-1 max-h-[24vh] overflow-auto rounded-lg bg-[#0d0f14] p-2 font-mono text-[10px] leading-relaxed text-slate-300">{playbook}</pre>
          </details>
        {/if}

        <!-- 观测配置 + Laya 经验沉淀(docs/14) -->
        <div class="mb-3">
          <div class="font-mono text-[10px] text-muted">
            观测 v2:{getPolicy().obsProfileCols} 列 / 前扫 {getPolicy().obsThreatsLookPx}px / pose
            {getPolicy().obsIncludePose ? 'on' : 'off'} / subgoal
            {getPolicy().obsIncludeSubgoal ? 'on' : 'off'}
          </div>
          {#if insights}
            <details class="mt-1">
              <summary class="cursor-pointer text-[11px] text-muted hover:text-white">
                Laya 使用经验({insights.split('\n').filter((l) => l.startsWith('- ')).length} 条 · insights.md)
              </summary>
              <pre
                class="mt-1 max-h-[24vh] overflow-auto rounded-lg bg-[#0d0f14] p-2 font-mono text-[10px] leading-relaxed text-violet-200/80">{insights}</pre>
            </details>
          {/if}
        </div>

        <!-- 审计尾 -->
        <div class="text-[10px] uppercase tracking-[0.08em] text-muted">
          审计尾 · evolution.jsonl
        </div>
        {#if auditTail.length > 0}
          <div class="mt-1 max-h-[20vh] overflow-auto font-mono text-[10px] leading-relaxed">
            {#each auditTail as a (a.ts + a.action)}
              <div class="text-slate-400">
                <span class="text-muted">#{a.iter}</span>
                <span
                  class={a.action === 'commit'
                    ? 'text-emerald-400'
                    : a.action === 'rollback'
                      ? 'text-red-400'
                      : a.action === 'propose'
                        ? 'text-fuchsia-300'
                        : 'text-muted'}>{a.actor}:{a.action}</span
                >
                {a.reason}
              </div>
            {/each}
          </div>
        {:else}
          <div class="mt-1 text-[11px] text-muted">尚无审计记录</div>
        {/if}
      </div>
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
          panel = panel === 'pm' ? null : 'pm';
        }}
        data-testid="btn-pm"
      >
        复盘 {postmortem ? '●' : '▸'}
      </button>
      <button
        class="ml-auto flex-none rounded-md border border-line px-2 py-0.5 text-[11px] text-muted hover:text-white"
        onclick={() => (panel = panel === 'laya' ? null : 'laya')}
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
