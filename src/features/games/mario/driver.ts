/**
 * 循环编排(docs/11 §2 差异②):决策与仿真异步解耦。
 *
 * - maybeDecide 按 intervalMs 节流,in-flight ≤1;决策快照带 attempt 守卫,
 *   解析回来时对局已死亡/重生 → 丢弃并记 stale(继承 Tetris pieceGen 教训)。
 * - 输入量子:决策结果转为 Input 持有,直到下一次决策替换;ESCALATE 时
 *   由规划器接管一个机动(autopilotInput),保证演示必达。
 * - 会话遥测:复用 laya-server /session/start + /session/log 落 JSONL
 *   (decision / event 行);sidecar 不在时静默降级,游戏照常。
 */

import { IDLE_INPUT, type GameState, type Input } from './engine';
import { actionInput, autopilotInput } from './planner';
import { getPolicy, type PolicyProfile } from './policy';
import {
  ENDPOINT_DEFAULT,
  MarioBrain,
  checkEndpoint,
  type Decision,
  type DriveMode,
} from './brain';
import {
  buildPostmortem,
  renderMarkdown,
  type PmDecisionRow,
  type PmEventRow,
  type PmSampleRow,
  type PostmortemReport,
  type SessionInput,
} from './postmortem';
import { TILE, type World } from './world1-1';

const LAYA_HEALTH = `${ENDPOINT_DEFAULT}/health`;
const QWEN_HEALTH = 'http://127.0.0.1:17230/health';

/** runId 序号:模块级单调递增,配合时间戳保证进程内唯一。 */
let runSeq = 0;

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export type DriverStats = {
  decisions: number;
  exec: number;
  reSense: number;
  escalate: number;
  stale: number;
  vetoes: number;
  avgConf: number;
  avgLatencyMs: number;
  qps: number;
};

export type LogRow = { t: string; cls: string; text: string };

export type SessionLogger = {
  start: (meta: Record<string, unknown>) => Promise<string | null>;
  push: (row: Record<string, unknown>) => void;
  flush: () => Promise<void>;
};

/** laya-server JSONL 会话(尽力而为:不可达时静默丢弃,游戏照常)。 */
export function createSessionLogger(game: string, fetchImpl: FetchLike = fetch): SessionLogger {
  let sessionId: string | null = null;
  let pending: Array<Record<string, unknown>> = [];
  let flushing = false;
  const logger: SessionLogger = {
    async start(meta): Promise<string | null> {
      await logger.flush();
      try {
        const r = await fetchImpl('http://127.0.0.1:10230/session/start', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ game, meta }),
        });
        const j = (await r.json()) as { id?: string };
        sessionId = j.id ?? null;
      } catch {
        sessionId = null;
      }
      return sessionId;
    },
    push(row): void {
      pending.push({ ts: Date.now(), ...row });
      if (pending.length >= 6) void logger.flush();
    },
    async flush(): Promise<void> {
      if (!sessionId || pending.length === 0 || flushing) return;
      flushing = true;
      const batch = pending;
      pending = [];
      try {
        await fetchImpl('http://127.0.0.1:10230/session/log', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ id: sessionId, entries: batch }),
        });
      } catch {
        pending = [...batch, ...pending]; // 失败回插队首,下次再试
      } finally {
        flushing = false;
      }
    },
  };
  return logger;
}

export class MarioDriver {
  mode: DriveMode = 'human';
  intervalMs = 250;
  layaReady = false;
  qwenReady = false;
  lastDecision: Decision | null = null;
  stats: DriverStats = {
    decisions: 0,
    exec: 0,
    reSense: 0,
    escalate: 0,
    stale: 0,
    vetoes: 0,
    avgConf: 0,
    avgLatencyMs: 0,
    qps: 0,
  };
  onDecision?: (d: Decision) => void;
  onLog?: (row: LogRow) => void;
  private brain: MarioBrain;
  private inFlight = false;
  private lastFire = 0;
  private startedAt = 0;
  private session: SessionLogger;
  /** 当前持有(动作量子);公开供 UI 显示与测试注入。 */
  held: Input = IDLE_INPUT;
  /** 跳跃弧补全:上一拍是否带 jump(见 inputFor)。 */
  private lastJump = false;
  /** 停滞检测:maxX 原地多久了(tick 数)。 */
  private lastMaxX = 0;
  private stallTicks = 0;
  /** 停滞自愈:剩余的规划器兜底拍数。 */
  private forcedEscalate = 0;
  /** 会话记录:复盘原料,与 JSONL 行同构(内存 ring,重开时清零)。 */
  private record: {
    id: string | null;
    decisions: PmDecisionRow[];
    events: PmEventRow[];
    samples: PmSampleRow[];
    final: SessionInput['final'];
    vetoes: number;
  } = { id: null, decisions: [], events: [], samples: [], final: undefined, vetoes: 0 };
  private lastSampleTick = 0;
  private lastVetoTick = -999;
  /**
   * 局内策略快照(docs/13 不变量①):beginRun 冻结,局内一律读快照,
   * Qwen/用户的更新只影响下一局。null = 未开局,回退全局 getPolicy()。
   */
  private runPolicy: PolicyProfile | null = null;
  /** 局标识:Qwen 分析幂等键(docs/13 §3.1),同一 runId 只分析一次。 */
  runId = '';

  constructor(opts: { endpoint?: string; fetchImpl?: FetchLike } = {}) {
    this.brain = new MarioBrain({ endpoint: opts.endpoint, fetchImpl: opts.fetchImpl });
    this.session = createSessionLogger('laya-mario', opts.fetchImpl);
  }

  configure(opts: {
    mode?: DriveMode;
    intervalMs?: number;
    gateExecute?: number;
    gateEscalate?: number;
  }): void {
    if (opts.mode !== undefined) {
      this.mode = opts.mode;
      this.brain.mode = opts.mode === 'pure' ? 'pure' : 'reflex';
    }
    if (opts.intervalMs !== undefined) this.intervalMs = opts.intervalMs;
    if (opts.gateExecute !== undefined) this.brain.gateExecute = opts.gateExecute;
    if (opts.gateEscalate !== undefined) this.brain.gateEscalate = opts.gateEscalate;
  }

  /**
   * 局前冻结策略快照并生成 runId(docs/13 §1)。必须在每局开局调用一次;
   * 局内 veto/规划器兜底全部读快照,保证复盘数据可归因。
   */
  beginRun(policy: PolicyProfile): void {
    this.runPolicy = { ...policy };
    runSeq += 1;
    this.runId = `r${Date.now().toString(36)}-${runSeq}`;
  }

  /** 局内策略读取的唯一出口:快照优先,未开局回退全局。 */
  private policy(): PolicyProfile {
    return this.runPolicy ?? getPolicy();
  }

  reset(): void {
    this.brain.reset();
    this.held = IDLE_INPUT;
    this.lastDecision = null;
    this.inFlight = false;
    this.lastFire = 0;
    this.startedAt = Date.now();
    this.record = {
      id: this.record.id,
      decisions: [],
      events: [],
      samples: [],
      final: undefined,
      vetoes: 0,
    };
    this.lastSampleTick = 0;
    this.stats = {
      ...this.stats,
      decisions: 0,
      exec: 0,
      reSense: 0,
      escalate: 0,
      stale: 0,
      vetoes: 0,
      avgConf: 0,
      avgLatencyMs: 0,
      qps: 0,
    };
  }

  startSession(meta: Record<string, unknown>): Promise<void> {
    return this.session.start({ runId: this.runId, mode: this.mode, ...meta }).then((id) => {
      this.record.id = id;
    });
  }

  /** 探活(只读,绝不代启;UI 状态点与模式门控共用)。 */
  async probe(): Promise<void> {
    const [laya, qwen] = await Promise.all([
      checkEndpoint(LAYA_HEALTH),
      checkEndpoint(QWEN_HEALTH),
    ]);
    this.layaReady = laya;
    this.qwenReady = qwen;
  }

  /**
   * 每 tick 调用:当前应持有的输入。human/autopilot 由 UI/规划器直接给出。
   *
   * 反射/纯反射的四条物理护栏(docs/11 §2 动作量子的已知缺口,实测教训):
   * 1. **跳跃弧补全**——单拍 250ms 会把上升期截断,gravityCut 把跳高锯断,
   *    Laya 再高置信也跳不过 h2 管道。上一拍带 jump 且仍在上升(vy<0)时,
   *    本拍延续 jump 直到过顶:与自驾 `jump = vy<0 || trigger` 同一不变量。
   * 2. **停滞自愈**——6s 无推进(maxX 不动)记 STALL 事件并强制规划器兜底
   *    12 拍(3s),把"高置信错误动作的死循环"变成可自愈的演示。
   * 3. **本能否决(veto)**——同排小怪 1 瓦内且本拍不跳:直冲必死。规划器
   *    机动接管一拍(踩/绕)。Laya 蒸馏前对近身威胁无反应,veto 计数是
   *    蒸馏效果的第一指标(应单调下降),不掩盖问题只保住游戏性。
   * 4. **采样**——每 300 tick(≈5s)记 maxX/分数快照,复盘时间线用。
   */
  inputFor(s: GameState, world: World): Input {
    if (this.mode !== 'reflex' && this.mode !== 'pure') return IDLE_INPUT;
    let held = this.held;
    if (this.lastJump && s.mario.vy < 0) held = { ...held, jump: true };
    if (s.phase === 'running') {
      if (s.maxX > this.lastMaxX) {
        this.lastMaxX = s.maxX;
        this.stallTicks = 0;
      } else {
        this.stallTicks += 1;
      }
      if (this.stallTicks >= 360) {
        this.stallTicks = 0;
        this.forcedEscalate = 12;
        this.log(
          'text-amber-400',
          `停滞 6s @ col ${Math.round(s.mario.x / TILE)} — 规划器兜底 ${this.forcedEscalate} 拍`,
        );
        this.event({
          event: 'stall',
          x: Math.round(s.mario.x),
          maxX: Math.round(s.maxX),
          tick: s.tick,
        });
      }
      // 本能否决:直冲小怪前抢一拍(计一次/episode,60 tick 去重)
      const P = this.policy();
      if (
        P.vetoEnabled &&
        !held.jump &&
        s.mario.vy >= 0 &&
        s.mario.onGround &&
        this.lethalGoombaAhead(s, P.vetoDistPx) &&
        s.tick - this.lastVetoTick >= 60
      ) {
        this.lastVetoTick = s.tick;
        this.held = autopilotInput(s, world, P);
        held = this.held;
        this.stats.vetoes += 1;
        this.record.vetoes += 1;
        this.log(
          'text-orange-300',
          `veto @ col ${Math.round(s.mario.x / TILE)} — 直冲小怪,接管一拍`,
        );
        this.event({ event: 'veto', x: Math.round(s.mario.x), tick: s.tick });
      }
      this.held = held;
      this.lastJump = held.jump;
      // 采样(复盘时间线)
      if (s.tick - this.lastSampleTick >= 300) {
        this.lastSampleTick = s.tick;
        this.record.samples.push({ tick: s.tick, maxX: s.maxX });
        this.record.final = {
          attempts: s.attempts,
          score: s.score,
          coins: s.coinCount,
          maxX: s.maxX,
          phase: s.phase,
        };
        this.session.push({
          type: 'sample',
          tick: s.tick,
          maxX: Math.round(s.maxX),
          attempts: s.attempts,
          score: s.score,
        });
      }
    }
    return held;
  }

  /** 同排(z 差 ≤8px)、身前 vetoDist px 内且存活激活的小怪。 */
  private lethalGoombaAhead(s: GameState, distPx: number): boolean {
    const front = s.mario.x + 14;
    for (const g of s.goombas) {
      if (!g.alive || !g.active) continue;
      if (Math.abs(g.y - s.mario.y) > 8) continue;
      const dx = g.x - front;
      if (dx > 0 && dx <= distPx) return true;
    }
    return false;
  }

  /** 节流发射决策(reflex/pure)。停滞自愈拍优先于 laya 请求。 */
  async maybeDecide(s: GameState, world: World, nowMs: number): Promise<void> {
    if (this.mode !== 'reflex' && this.mode !== 'pure') return;
    if (nowMs - this.lastFire < this.intervalMs) return;
    if (this.forcedEscalate > 0) {
      this.forcedEscalate -= 1;
      this.lastFire = nowMs;
      this.held = autopilotInput(s, world);
      this.lastJump = this.held.jump;
      this.stats.decisions += 1;
      this.stats.escalate += 1;
      this.record.decisions.push({
        action: 'guard',
        conf: 0,
        gate: 'ESCALATE',
        col: Math.round((s.mario.x / TILE) * 10) / 10,
        applied: true,
        note: 'stall-guard',
      });
      if (this.forcedEscalate % 4 === 0) {
        this.log('text-amber-400', `兜底机动(剩 ${this.forcedEscalate} 拍)`);
      }
      return;
    }
    if (this.inFlight || !this.layaReady) return;
    this.inFlight = true;
    this.lastFire = nowMs;
    const attempt = s.attempts;
    try {
      const d = await this.brain.decide(s, world);
      if (s.attempts !== attempt || s.phase !== 'running') {
        this.stats.stale += 1;
        this.log('text-slate-500', `stale 决策丢弃 attempt=${attempt}`);
        this.record.decisions.push({
          action: d.action,
          conf: d.conf,
          gate: d.gate,
          latencyMs: Math.round(d.latencyMs),
          applied: false,
          note: 'stale',
        });
        this.session.push({
          type: 'decision',
          attempt,
          mode: this.mode,
          action: d.action,
          conf: d.conf,
          gate: d.gate,
          latencyMs: Math.round(d.latencyMs),
          applied: false,
          note: 'stale',
        });
        return;
      }
      this.lastDecision = d;
      this.held = d.escalateManeuver ? autopilotInput(s, world) : actionInput(d.action);
      const n = this.stats.decisions + 1;
      this.stats.decisions = n;
      if (d.gate === 'EXECUTE') this.stats.exec += 1;
      else if (d.gate === 'RE_SENSE') this.stats.reSense += 1;
      else this.stats.escalate += 1;
      this.stats.avgConf = this.stats.avgConf + (d.conf - this.stats.avgConf) / n;
      this.stats.avgLatencyMs =
        this.stats.avgLatencyMs + (d.latencyMs - this.stats.avgLatencyMs) / n;
      this.stats.qps =
        Math.round((n / Math.max(1, (Date.now() - this.startedAt) / 1000)) * 10) / 10;
      this.onDecision?.(d);
      this.record.decisions.push({
        action: d.action,
        conf: d.conf,
        gate: d.gate,
        latencyMs: Math.round(d.latencyMs),
        col: Math.round((s.mario.x / TILE) * 10) / 10,
        applied: true,
      });
      this.session.push({
        type: 'decision',
        attempt,
        mode: this.mode,
        action: d.action,
        conf: d.conf,
        gate: d.gate,
        latencyMs: Math.round(d.latencyMs),
        probs: d.probs,
        candidates: d.candidates,
        note: d.note,
        applied: true,
        sensed: d.sensed,
        col: Math.round(s.mario.x),
      });
    } catch (e) {
      this.layaReady = false; // 停止发射,等 UI 探活恢复
      this.onLog?.({
        t: now(),
        cls: 'text-red-400',
        text: `laya-server 不可达:${e instanceof Error ? e.message : String(e)}`,
      });
    } finally {
      this.inFlight = false;
    }
  }

  log(cls: string, text: string): void {
    this.onLog?.({ t: now(), cls, text });
  }

  event(row: Record<string, unknown>): void {
    this.session.push({ type: 'event', ...row });
    // 本地记录:type 归一为事件种类(death/stall/win/veto/respawn)
    this.record.events.push({
      ...(row as PmEventRow),
      type: String((row as { event?: string }).event ?? 'unknown'),
    });
  }

  /** 生成复盘(规则判定,无 LLM):结构化报告 + 固定格式 Markdown。 */
  buildPostmortem(): { report: PostmortemReport; markdown: string } {
    const input: SessionInput = {
      id: this.record.id ?? `${this.mode}-${this.startedAt}`,
      mode: this.mode,
      decisions: this.record.decisions,
      events: this.record.events,
      samples: this.record.samples,
      final: this.record.final,
      vetoes: this.record.vetoes,
    };
    const report = buildPostmortem(input);
    return { report, markdown: renderMarkdown(report) };
  }

  /** 复盘文档落入会话 JSONL(单行转义),供离线批扫与 Qwen/编码代理消费。 */
  pushPostmortem(markdown: string): void {
    this.session.push({ type: 'postmortem', md: markdown });
    void this.session.flush();
  }

  /** 会话收尾快照(头面运行/测试显式收口;UI 由采样自动维护)。 */
  markFinal(final: {
    attempts?: number;
    score?: number;
    coins?: number;
    maxX?: number;
    phase?: string;
  }): void {
    this.record.final = final;
  }

  flush(): Promise<void> {
    return this.session.flush();
  }
}

function now(): string {
  const d = new Date();
  return `${d.toLocaleTimeString('zh-CN', { hour12: false })}.${String(d.getMilliseconds()).padStart(3, '0')}`;
}
