/**
 * 自主进化闭环(docs/13 + docs/15 v3,本地侧;全程不经云端)。
 *
 *   训练局(champion,Laya 反射)
 *     → 规则复盘(postmortem.ts,含过程异常:首死/空转/门控抖动)
 *     → 触发器(docs/15 §6.1:死亡级分析清单,deathKey=runId#attempt 幂等)
 *     → 分析队列(串行、同 key 合并、溢出落盘 backlog、超时+熔断)
 *     → Qwen v3(手册 + 沙箱补丁[单维度] + death_diagnosis + 可证伪 insight)
 *     → sanitize → 限幅(单维度混车丢弃)→ 冷却期 → **run 级合并候选**
 *   候选按 patchKind 选评估窗(数值 K=3 / 设计 K=5)中位数对比 champion:
 *   过 +ε 转正(记账本+版本),否则回滚+冷却+写回证伪。退化自动回退最优版本。
 *
 * 用法:npx tsx e2e/mario-loop.ts [局数=5] [--mode=reflex|autopilot]
 *       [--dir=<状态目录>] [--eval-runs=3]
 * 默认状态目录:~/.qianxun_dev/mario(与 UI 共享 champion);旧 var/mario 自动迁移。
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { MarioDriver, type DriverStats, type SentRow } from '../src/features/games/mario/driver';
import { createGameState, step, type GameState } from '../src/features/games/mario/engine';
import {
  DESIGN_EVAL_RUNS,
  EVAL_RUNS_DEFAULT,
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
  detectRegression,
  diffPatch,
  evalMedian,
  judgeCandidate,
  limitPatch,
  markAnalyzed,
  noteProposals,
  obsSnapshotOf,
  parseEvoState,
  parseHistory,
  parsePolicyFile,
  planAnalyses,
  recordLedger,
  recordRun,
  recordVersion,
  restoreVersion,
  startCooldowns,
  wrapPolicy,
  type AuditEntry,
  type ChangeLedgerRow,
  type EvoState,
  type PersistTask,
  type PolicyEnvelope,
  type QwenCounters,
  type RunOutcome,
} from '../src/features/games/mario/evolution';
import type { PostmortemReport, SessionInput } from '../src/features/games/mario/postmortem';
import { getPolicy, setPolicy, type PolicyProfile } from '../src/features/games/mario/policy';
import {
  PLAYBOOK_MAX_CHARS,
  buildDeathPayload,
  refineViaQwen,
  type DeathDiagnosis,
  type HistoryRow,
  type LayaInsight,
  type ParseRefineResult,
} from '../src/features/games/mario/qwen';
import {
  CircuitBreaker,
  nextTask,
  removeTask,
  scheduleTask,
  taskFailed,
  type SchedTask,
} from '../src/features/games/mario/machine';
import { autopilotInput } from '../src/features/games/mario/planner';
import { WORLD_1_1, type World } from '../src/features/games/mario/world1-1';

const TICK_MS = 1000 / 60;
const MAX_TICKS = 60 * 90;

export type IterationResult = {
  iteration: number;
  mode: string;
  won: boolean;
  attempts: number;
  maxXCol: number;
  ticks: number;
  decisions: number;
  execRate: number;
  vetoes: number;
  wallMs: number;
  score: number;
  /** 本轮触发的分析任务(原因串)。 */
  trigger: string;
  /** 本轮候选评估结论(若有候选在评)。 */
  verdict?: 'commit' | 'rollback';
  policyIssues: string[];
  playbookChars: number;
  error?: string;
};

export type LoopState = {
  envelope: PolicyEnvelope;
  playbook: string;
  /** Laya 使用经验沉淀(docs/14 §6,insights.md)。 */
  insights: string;
  history: HistoryRow[];
  evo: EvoState;
};

// ---------- 状态持久化(docs/13 §5:与 UI 同一数据根、同一布局) ----------

/** 默认数据根:与 Tauri debug 构建一致,UI 与闭环共享 champion。 */
export function defaultDataDir(): string {
  return path.join(os.homedir(), '.qianxun_dev', 'mario');
}

/**
 * 旧 var/mario 一次性迁入新数据根(仅当新根还没有 policy.json)。
 * 只迁手册与历史;**不迁旧策略**——旧 champion 的窗口语义来自重构前的
 * 规划器,直接继承会让自驾卡死(champion 从 DEFAULT_POLICY 重新出发)。
 */
export function migrateLegacyDir(dir: string, log: (line: string) => void): void {
  const legacy = path.resolve('var', 'mario');
  if (
    path.resolve(dir) === legacy ||
    fs.existsSync(path.join(dir, POLICY_FILE)) ||
    !fs.existsSync(path.join(legacy, PLAYBOOK_FILE))
  ) {
    return;
  }
  fs.mkdirSync(dir, { recursive: true });
  const legacyPlaybook = path.join(legacy, PLAYBOOK_FILE);
  if (fs.existsSync(legacyPlaybook)) fs.copyFileSync(legacyPlaybook, path.join(dir, PLAYBOOK_FILE));
  const oldHistory = path.join(legacy, 'evolution.json');
  if (fs.existsSync(oldHistory)) fs.copyFileSync(oldHistory, path.join(dir, HISTORY_FILE));
  log(`已迁移旧状态 ${legacy} → ${dir}(仅手册/历史;champion 回默认重新进化)`);
}

function readText(dir: string, name: string): string | null {
  try {
    return fs.readFileSync(path.join(dir, name), 'utf8');
  } catch {
    return null;
  }
}

/** 原子写:tmp + rename(docs/15 R7——撕裂的 state 会连锁清掉基线)。 */
function writeAtomic(file: string, data: string): void {
  const tmp = `${file}.tmp`;
  fs.writeFileSync(tmp, data);
  fs.renameSync(tmp, file);
}

export function loadState(dir: string): LoopState {
  const { envelope } = parsePolicyFile(readText(dir, POLICY_FILE));
  const playbook = readText(dir, PLAYBOOK_FILE) ?? '';
  const insights = readText(dir, INSIGHTS_FILE) ?? '';
  // 历史:history.json 优先;旧 evolution.json 兼容读
  let history = parseHistory(readText(dir, HISTORY_FILE));
  if (history.length === 0) history = parseHistory(readText(dir, 'evolution.json'));
  const { state: evo } = parseEvoState(readText(dir, STATE_FILE));
  setPolicy(envelope.policy);
  return { envelope, playbook, insights, history, evo };
}

export function saveState(dir: string, state: LoopState): void {
  fs.mkdirSync(dir, { recursive: true });
  const envelope: PolicyEnvelope = { ...state.envelope, updatedAt: new Date().toISOString() };
  state = { ...state, envelope };
  writeAtomic(path.join(dir, POLICY_FILE), JSON.stringify(envelope, null, 2));
  writeAtomic(path.join(dir, PLAYBOOK_FILE), state.playbook.slice(0, PLAYBOOK_MAX_CHARS));
  writeAtomic(path.join(dir, INSIGHTS_FILE), state.insights);
  writeAtomic(path.join(dir, HISTORY_FILE), JSON.stringify(state.history, null, 2));
  writeAtomic(path.join(dir, STATE_FILE), JSON.stringify(state.evo, null, 2));
}

/** 审计只追加:每条规则变更的完整证据链(docs/13 §5;先审计后 state 写序)。 */
function appendAudit(dir: string, entry: AuditEntry): void {
  fs.mkdirSync(dir, { recursive: true });
  fs.appendFileSync(path.join(dir, EVOLUTION_FILE), auditLine(entry) + '\n');
}

// ---------- 头面会话:确定性引擎 + 真实/虚拟节奏 ----------

export type HeadlessResult = {
  won: boolean;
  ticks: number;
  stats: DriverStats;
  report: PostmortemReport;
  runId: string;
  /** 分析载荷原料(docs/15 §6.3):会话快照 + Laya 收发明细(内存浅拷贝)。 */
  session: SessionInput;
  sent: SentRow[];
};

export async function runHeadlessSession(opts: {
  mode: 'reflex' | 'autopilot';
  maxTicks?: number;
  /** 本局策略快照(docs/13 不变量①);不传则读全局。 */
  policy?: PolicyProfile;
}): Promise<HeadlessResult> {
  const world: World = WORLD_1_1;
  const s: GameState = createGameState(world);
  const policy = opts.policy ?? getPolicy();
  const driver = new MarioDriver();
  driver.beginRun(policy);
  driver.configure({
    mode: opts.mode,
    intervalMs: policy.intervalMs,
    gateExecute: policy.gateExecute,
    gateEscalate: policy.gateEscalate,
  });
  if (opts.mode === 'reflex') {
    await driver.probe();
    if (!driver.layaReady) throw new Error('laya-server 不可达(:10230)——反射模式需要本地服务');
  }
  const maxTicks = opts.maxTicks ?? MAX_TICKS;
  const beatTicks = Math.max(1, Math.round(policy.intervalMs / TICK_MS));
  let virtualNow = 0;
  let won = false;
  while (s.tick < maxTicks && s.phase !== 'won') {
    if (opts.mode === 'reflex') {
      virtualNow += policy.intervalMs;
      await driver.maybeDecide(s, world, virtualNow);
    }
    for (let k = 0; k < beatTicks && s.tick < maxTicks && s.phase !== 'won'; k += 1) {
      const input =
        opts.mode === 'autopilot' ? autopilotInput(s, world, policy) : driver.inputFor(s, world);
      const events = step(world, s, input, 384);
      for (const e of events) {
        if (e.type === 'death') {
          driver.event({
            event: 'death',
            cause: e.cause,
            x: Math.round(s.mario.x),
            tick: s.tick,
            attempt: s.attempts,
          });
        } else if (e.type === 'win') {
          won = true;
          driver.event({ event: 'win', score: s.score, attempt: s.attempts });
        } else if (e.type === 'respawn') {
          driver.event({ event: 'respawn', attempt: s.attempts });
        }
      }
    }
  }
  driver.markFinal({
    attempts: s.attempts,
    score: s.score,
    coins: s.coinCount,
    maxX: s.maxX,
    phase: s.phase,
  });
  const { report } = driver.buildPostmortem();
  return {
    won,
    ticks: s.tick,
    stats: { ...driver.stats },
    report,
    runId: driver.runId,
    session: driver.sessionSnapshot(),
    sent: [...driver.sentLog],
  };
}

/** 局后分析(委托共享客户端 qwen.ts;测试注入 fetchImpl)。 */
export async function refineWithQwen(input: {
  playbook: string;
  report: PostmortemReport;
  policy: PolicyProfile;
  history: HistoryRow[];
  reason?: string;
  deathPayload?: string;
  lastDiagnosis?: DeathDiagnosis[];
  prevPatch?: Record<string, { from: unknown; to: unknown }>;
  lastInsights?: string[];
  validDeathKeys?: readonly string[];
  endpoint?: string;
  signal?: AbortSignal;
  fetchImpl?: typeof fetch;
}): Promise<ParseRefineResult> {
  return refineViaQwen(input);
}

// ---------- 主循环:候选—评估—提交(docs/13 §4 + docs/15 §6) ----------

type PendingCandidate = {
  policy: PolicyProfile;
  patch: Record<string, { from: unknown; to: unknown }>;
  issues: string[];
  reason: string;
  kind: 'numeric' | 'design';
  originSigs: string[];
};

function withQwenCounter(evo: EvoState, k: keyof QwenCounters, n = 1): EvoState {
  return {
    ...evo,
    counters: { ...evo.counters, qwen: { ...evo.counters.qwen, [k]: evo.counters.qwen[k] + n } },
  };
}

function persistToTask(t: PersistTask): SchedTask {
  const priority = t.kind === 'death' ? (t.attempt === 1 ? 0 : 2) : t.kind === 'win' ? 3 : 4;
  return {
    key: t.key,
    kind: t.kind,
    attempt: t.attempt,
    sig: t.sig ?? null,
    reason: t.reason,
    priority,
  };
}

function taskToPersist(t: SchedTask): PersistTask {
  return {
    key: t.key,
    kind: t.kind,
    attempt: t.attempt,
    sig: t.sig ?? null,
    reason: t.reason,
    ts: new Date().toISOString(),
  };
}

function insightText(ins: LayaInsight): string {
  if (ins.kind === 'claim') {
    const tag = `${ins.metric ?? ''}${ins.direction === 'down' ? '↓' : '↑'}${ins.field ? ` ${ins.field}` : ''}`;
    return `${ins.claim} [${tag.trim()}]`;
  }
  return ins.claim;
}

export async function runLoop(opts: {
  iterations: number;
  mode: 'reflex' | 'autopilot';
  dataDir: string;
  /** 候选试跑局数(中位数聚合),默认 EVAL_RUNS_DEFAULT;设计变更窗加倍。 */
  evalRuns?: number;
  log?: (line: string) => void;
}): Promise<IterationResult[]> {
  const log = opts.log ?? ((line: string) => console.log(line));
  const K = Math.max(1, opts.evalRuns ?? EVAL_RUNS_DEFAULT);
  const state = loadState(opts.dataDir);
  const results: IterationResult[] = [];
  let pending: PendingCandidate | null = null;
  const breaker = new CircuitBreaker();
  // 诊断/补丁链(内存):death-repeat 载荷⑥的"上轮为何没生效"素材
  const diagBySig = new Map<string, DeathDiagnosis[]>();
  const patchBySig = new Map<string, Record<string, { from: unknown; to: unknown }>>();

  const evalK = async (policy: PolicyProfile, runs: number): Promise<RunOutcome[]> => {
    const out: RunOutcome[] = [];
    for (let j = 0; j < runs; j += 1) {
      const r = await runHeadlessSession({ mode: opts.mode, maxTicks: MAX_TICKS, policy });
      out.push({ won: r.won, maxXCol: r.report.maxXCol, ticks: r.ticks });
    }
    return out;
  };

  for (let i = 1; i <= opts.iterations; i += 1) {
    const t0 = Date.now();
    let verdict: IterationResult['verdict'];
    let triggerReason = 'autopilot(教师采集,不调 Qwen)';
    let allAnalysesOk = true;
    let hadTasks = false;
    let policyIssues: string[] = [];
    let run: HeadlessResult | null = null;
    let recorded = false;
    try {
      // ① 评估上一轮候选:测量通过才转正(LLM 负责创意,代码负责纪律)
      if (pending !== null) {
        const iter = state.evo.iteration;
        const window = pending.kind === 'design' ? DESIGN_EVAL_RUNS : K;
        if (state.evo.championScore === null) {
          const base = await evalK(state.envelope.policy, window);
          state.evo.championScore = evalMedian(base);
          appendAudit(opts.dataDir, {
            v: 1,
            ts: new Date().toISOString(),
            iter,
            runId: null,
            actor: 'system',
            action: 'evaluate',
            reason: `champion 基线测量(N=${window})`,
            score: { champion: state.evo.championScore, candidate: null },
          });
          log(`[局 ${i}] champion 基线:中位分 ${state.evo.championScore.toFixed(1)}(N=${window})`);
        }
        const cRuns = await evalK(pending.policy, window);
        const cScore = evalMedian(cRuns);
        const v = judgeCandidate(state.evo.championScore, cScore);
        verdict = v.verdict;
        const delta = state.evo.championScore !== null ? cScore - state.evo.championScore : null;
        const ledgerRow: ChangeLedgerRow = {
          changeId: `c-${iter}`,
          vid: state.evo.championVid,
          fields: pending.patch,
          kind: pending.kind,
          dimension: pending.kind === 'design' ? 'message-design' : 'gate',
          originSigs: pending.originSigs,
          deathKeys: [],
          verdict: v.verdict,
          scoreDelta: delta,
        };
        if (v.verdict === 'commit') {
          state.envelope = wrapPolicy(pending.policy, `commit:iter-${iter}`);
          state.evo.championScore = cScore;
          state.evo = withQwenCounter(state.evo, 'commits');
          state.evo = recordVersion(state.evo, {
            policy: pending.policy,
            patch: pending.patch,
            originSigs: pending.originSigs,
            verdict: 'commit',
            score: cScore,
            iter,
          });
          setPolicy(pending.policy);
        } else {
          state.evo.cooldowns = startCooldowns(
            state.evo.cooldowns,
            Object.keys(pending.patch),
            iter,
          );
          state.evo = withQwenCounter(state.evo, 'rollbacks');
          state.evo = recordVersion(state.evo, {
            policy: pending.policy,
            patch: pending.patch,
            originSigs: pending.originSigs,
            verdict: 'rollback',
            score: cScore,
            iter,
          });
          // 回滚写回证伪(docs/15 §6.7):手册追加"勿重复提案"
          state.playbook = appendInsight(
            state.playbook,
            iter,
            `假设已回滚(Δ=${delta?.toFixed(1) ?? '?'}):${Object.keys(pending.patch).join(',')} 勿重复提案`,
          );
        }
        state.evo = recordLedger(state.evo, ledgerRow);
        appendAudit(opts.dataDir, {
          v: 1,
          ts: new Date().toISOString(),
          iter,
          runId: null,
          actor: 'system',
          action: v.verdict,
          reason: v.reason,
          patch: pending.patch,
          patchKind: pending.kind,
          score: { champion: state.evo.championScore, candidate: cScore },
          issues: pending.issues,
        });
        log(
          `[局 ${i}] 候选 ${v.verdict === 'commit' ? '转正' : '回滚'}:${v.reason}` +
            (pending.issues.length > 0 ? `(纪律:${pending.issues.join(';')})` : ''),
        );
        pending = null;
        saveState(opts.dataDir, state);
      }

      // ② 训练局:champion 快照开局(局内不可变)
      const champion = state.envelope.policy;
      run = await runHeadlessSession({
        mode: opts.mode,
        maxTicks: MAX_TICKS,
        policy: champion,
      });
      state.history = appendHistory(state.history, {
        iteration: state.evo.iteration + 1,
        won: run.won,
        maxXCol: run.report.maxXCol,
        ticks: run.ticks,
      });

      // ③ 死亡级分析队列(docs/15 §6.1/6.2):串行、合并、落盘 backlog
      if (opts.mode === 'reflex') {
        const tasks = planAndEnqueue(state.evo, run.runId, run.report);
        hadTasks = tasks.total > 0;
        triggerReason = tasks.enqueued.map((t) => t.reason).join(',') || 'no-signal(无死亡无通关)';
        let queue = tasks.enqueued;
        let backlog = state.evo.pendingAnalyses.map(persistToTask);
        const iter = state.evo.iteration + 1;
        const obs = obsSnapshotOf(champion);
        const validKeys = [...queue, ...backlog].map((t) => t.key);
        const accPatch: Record<string, { from: unknown; to: unknown }> = {};
        const accSigs = new Set<string>();
        policyIssues = [];
        if (hadTasks) log(`[局 ${i}] 分析任务 ${queue.length + backlog.length} 项`);
        for (;;) {
          if (breaker.tripped) {
            log(`[局 ${i}] 熔断中,剩余任务留 backlog`);
            break;
          }
          const task = nextTask(backlog, queue);
          if (task === null) break;
          backlog = removeTask(backlog, task.key);
          queue = removeTask(queue, task.key);
          appendAudit(opts.dataDir, {
            v: 1,
            ts: new Date().toISOString(),
            iter,
            runId: run.runId,
            actor: 'qwen',
            action: 'analyze',
            phase: 'start',
            key: task.key,
            reason: task.reason,
          });
          const tA = Date.now();
          try {
            const death =
              task.kind === 'death'
                ? run.report.deaths.find((d) => d.attempt === task.attempt)
                : undefined;
            let payload: string | undefined;
            if (task.kind === 'death' && death) {
              payload = buildDeathPayload({
                report: run.report,
                session: run.session,
                sent: run.sent,
                attempt: task.attempt ?? 1,
                cause: death.cause,
                landmark: death.landmark,
              }).text;
            }
            const refined = await refineWithQwen({
              playbook: state.playbook,
              report: run.report,
              policy: champion,
              history: state.history,
              reason: task.reason,
              deathPayload: payload,
              lastDiagnosis: task.sig ? (diagBySig.get(task.sig) ?? []) : [],
              prevPatch: task.sig ? patchBySig.get(task.sig) : undefined,
              lastInsights: state.insights
                .split('\n')
                .filter((l) => l.trim() !== '')
                .slice(-3),
              validDeathKeys: validKeys,
            });
            // 成功才占幂等键(docs/15 R6:失败不占,任务可重试)
            state.evo = markAnalyzed(state.evo, {
              key: task.key,
              kind: task.kind,
              attempt: task.attempt,
              sig: task.sig,
              reason: task.reason,
              priority: task.priority,
            });
            state.evo = withQwenCounter(
              state.evo,
              task.kind === 'death' ? 'deathAnalyses' : 'runAnalyses',
            );
            breaker.ok();
            appendAudit(opts.dataDir, {
              v: 1,
              ts: new Date().toISOString(),
              iter,
              runId: run.runId,
              actor: 'qwen',
              action: 'analyze',
              phase: 'ok',
              key: task.key,
              reason: task.reason,
              latencyMs: Date.now() - tA,
            });
            // 手册/insight 立即沉淀(可证伪;教训不回滚但可被证伪)
            state.playbook = refined.playbook;
            if (refined.insight !== null) {
              state.insights = appendInsight(
                state.insights,
                iter,
                insightText(refined.insight),
                obs,
              );
              log(`[局 ${i}] 经验+1:${refined.insight.claim.slice(0, 24)}`);
            } else {
              state.evo = withQwenCounter(state.evo, 'emptyInsight');
              if (refined.insightIssue !== '') policyIssues.push(refined.insightIssue);
            }
            if (refined.issues.some((x) => x.includes('不含 JSON'))) {
              state.evo = withQwenCounter(state.evo, 'badJson');
            }
            // 诊断链记账(death-repeat 的"上轮为何没生效"素材)
            if (task.sig && refined.diagnosis.length > 0) {
              diagBySig.set(task.sig, [...(diagBySig.get(task.sig) ?? []), ...refined.diagnosis]);
            }
            // 补丁累积(run 级合并提案):同 run 多次分析 → 一个候选
            const limited = limitPatch(refined.policy, champion);
            const cooled = applyCooldowns(limited.policy, champion, state.evo.cooldowns, iter);
            Object.assign(accPatch, diffPatch(champion, cooled.policy));
            policyIssues.push(...refined.issues, ...limited.issues);
            if (task.sig && Object.keys(accPatch).length > 0) {
              accSigs.add(task.sig);
              patchBySig.set(task.sig, { ...accPatch });
            }
          } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            const isTimeout = /timeout|abort/i.test(msg);
            state.evo = withQwenCounter(state.evo, isTimeout ? 'timeouts' : 'failed');
            breaker.fail();
            const retry = taskFailed(task);
            appendAudit(opts.dataDir, {
              v: 1,
              ts: new Date().toISOString(),
              iter,
              runId: run.runId,
              actor: 'qwen',
              action: 'analyze',
              phase: 'fail',
              key: task.key,
              reason: task.reason,
              errClass: isTimeout ? 'timeout' : 'error',
              latencyMs: Date.now() - tA,
            });
            if (retry !== null) {
              state.evo = withQwenCounter(state.evo, 'retries');
              queue = [...queue, retry];
              log(`[局 ${i}] 分析失败(${msg}),重试 ${retry.attempts}/2`);
            } else {
              allAnalysesOk = false;
              log(`[局 ${i}] 分析放弃:${task.key} ${msg}`);
            }
          }
        }
        // 剩余未处理任务落盘 backlog(不丢死亡)
        state.evo.pendingAnalyses = [...backlog, ...queue].map(taskToPersist);
        // run 级合并提案
        if (Object.keys(accPatch).length > 0) {
          const patchPolicy = {
            ...champion,
            ...Object.fromEntries(Object.entries(accPatch).map(([k, v]) => [k, v.to])),
          } as PolicyProfile;
          const limited = limitPatch(patchPolicy, champion);
          const cooled = applyCooldowns(limited.policy, champion, state.evo.cooldowns, iter);
          const patch = diffPatch(champion, cooled.policy);
          policyIssues.push(...limited.issues);
          if (Object.keys(patch).length > 0) {
            state.evo = withQwenCounter(state.evo, 'patches');
            state.evo = noteProposals(state.evo, [...accSigs], iter);
            pending = {
              policy: cooled.policy,
              patch,
              issues: policyIssues,
              reason: triggerReason,
              kind: limited.kind === 'design' ? 'design' : 'numeric',
              originSigs: [...accSigs],
            };
            appendAudit(opts.dataDir, {
              v: 1,
              ts: new Date().toISOString(),
              iter,
              runId: run.runId,
              actor: 'qwen',
              action: 'propose',
              reason: triggerReason,
              patch,
              patchKind: pending.kind,
              obs,
              issues: policyIssues,
            });
            log(
              `[局 ${i}] 提案 ${Object.keys(patch).length} 字段(${pending.kind},评估窗 ` +
                `${pending.kind === 'design' ? DESIGN_EVAL_RUNS : K} 局)`,
            );
          } else {
            appendAudit(opts.dataDir, {
              v: 1,
              ts: new Date().toISOString(),
              iter,
              runId: run.runId,
              actor: 'qwen',
              action: 'skip',
              reason: '补丁为空或与 champion 无差异',
              obs,
              issues: policyIssues,
            });
            log(`[局 ${i}] 补丁无有效变更(手册仍更新)`);
          }
        } else if (!hadTasks) {
          appendAudit(opts.dataDir, {
            v: 1,
            ts: new Date().toISOString(),
            iter,
            runId: run.runId,
            actor: 'system',
            action: 'skip',
            reason: triggerReason,
          });
        }
      }

      // ④ 局末簿记(幂等标记已按任务成功后占;recordRun 记计数与签名统计)
      state.evo = recordRun(state.evo, run.report, run.runId, hadTasks && allAnalysesOk, opts.mode);
      recorded = true;
      saveState(opts.dataDir, state);

      // ⑤ 回归守卫(docs/15 §7):连续退化 → 自动回退最优版本
      const reg = detectRegression(state.history);
      if (reg !== null) {
        const best = state.evo.versions
          .filter((v) => v.verdict === 'commit')
          .sort((a, b) => b.score - a.score)[0];
        if (best && best.vid !== state.evo.championVid) {
          const r = restoreVersion(state.evo, best.vid);
          if (r !== null) {
            state.evo = r.state;
            state.envelope = wrapPolicy(r.policy, `restore:v${best.vid}`);
            setPolicy(r.policy);
            appendAudit(opts.dataDir, {
              v: 1,
              ts: new Date().toISOString(),
              iter: state.evo.iteration,
              runId: run.runId,
              actor: 'system',
              action: 'restore',
              reason: `回归(近10局胜率 ${reg.current.toFixed(2)} < 最优 ${reg.best.toFixed(2)}×0.5),回退 v${best.vid}`,
            });
            log(`[局 ${i}] 回归守卫:回退 v${best.vid}`);
          }
        }
      }

      results.push({
        iteration: i,
        mode: opts.mode,
        won: run.won,
        attempts: run.report.attempts,
        maxXCol: run.report.maxXCol,
        ticks: run.ticks,
        decisions: run.stats.decisions,
        execRate: run.stats.decisions > 0 ? run.stats.exec / run.stats.decisions : 0,
        vetoes: run.stats.vetoes,
        wallMs: Date.now() - t0,
        score: evalMedian([{ won: run.won, maxXCol: run.report.maxXCol, ticks: run.ticks }]),
        trigger: triggerReason,
        verdict,
        policyIssues,
        playbookChars: state.playbook.length,
      });
    } catch (e) {
      // 连带伤害修复(docs/15 R1):局已跑完但后续失败,簿记照记
      if (run !== null && !recorded) {
        try {
          state.evo = recordRun(state.evo, run.report, run.runId, false, opts.mode);
        } catch {
          /* 簿记失败不掩盖原始错误 */
        }
      }
      results.push({
        iteration: i,
        mode: opts.mode,
        won: false,
        attempts: 0,
        maxXCol: 0,
        ticks: 0,
        decisions: 0,
        execRate: 0,
        vetoes: 0,
        wallMs: Date.now() - t0,
        score: 0,
        trigger: triggerReason,
        policyIssues,
        playbookChars: state.playbook.length,
        error: e instanceof Error ? e.message : String(e),
      });
      log(`[局 ${i}] 失败:${results[results.length - 1]?.error ?? 'unknown'}`);
      saveState(opts.dataDir, state);
    }
  }
  return results;

  /** 触发器 + 入队(docs/15 §6.1/6.2):planAnalyses → scheduleTask,溢出计数。 */
  function planAndEnqueue(
    evo: EvoState,
    runId: string,
    report: PostmortemReport,
  ): { enqueued: SchedTask[]; total: number } {
    const tasks = planAnalyses(evo, report, runId);
    let queue: SchedTask[] = [];
    for (const t of tasks) {
      const r = scheduleTask(queue, {
        key: t.key,
        kind: t.kind,
        attempt: t.attempt,
        sig: t.sig,
        reason: t.reason,
        priority: t.priority,
      });
      queue = r.queue;
      if (r.merged) evo.counters.qwen.merged += 1;
      if (r.overflow !== null) {
        evo.counters.qwen.droppedByCap += 1; // 落盘 backlog 不丢:由调用方收尾 persist
      }
    }
    return { enqueued: queue, total: tasks.length };
  }
}

async function main(): Promise<void> {
  const iterations = Number(process.argv[2] ?? 5) || 5;
  const modeArg = process.argv.find((a) => a.startsWith('--mode='))?.slice(7);
  const mode = (modeArg === 'autopilot' ? 'autopilot' : 'reflex') as 'reflex' | 'autopilot';
  const dirArg = process.argv.find((a) => a.startsWith('--dir='))?.slice(6);
  const dataDir = path.resolve(dirArg ?? defaultDataDir());
  const evalRuns =
    Number(process.argv.find((a) => a.startsWith('--eval-runs='))?.slice(12)) || undefined;
  migrateLegacyDir(dataDir, (l) => console.log(l));
  console.log(
    `=== 自主进化闭环 · ${iterations} 局 · ${mode} · K=${evalRuns ?? EVAL_RUNS_DEFAULT} · ${dataDir} ===`,
  );
  const results = await runLoop({ iterations, mode, dataDir, evalRuns });
  const table = results
    .map(
      (r) =>
        `#${r.iteration} ${r.won ? '★' : '×'} maxX=${r.maxXCol}col ticks=${r.ticks} ` +
        `score=${r.score.toFixed(0)} veto=${r.vetoes} exec=${(r.execRate * 100).toFixed(0)}% ` +
        `${r.verdict ? `[候选${r.verdict === 'commit' ? '转正' : '回滚'}] ` : ''}` +
        `${r.error ? `err=${r.error}` : `[${r.trigger}]`}`,
    )
    .join('\n');
  console.log(table);
  const completed = results.filter((r) => r.won);
  const playbook = fs.existsSync(path.join(dataDir, PLAYBOOK_FILE))
    ? fs.readFileSync(path.join(dataDir, PLAYBOOK_FILE), 'utf8')
    : '';
  console.log(
    `=== 总结:通关率 ${((completed.length / Math.max(1, results.length)) * 100).toFixed(0)}%` +
      `${completed.length > 0 ? ` · 最快通关 ${Math.min(...completed.map((r) => r.ticks))} ticks` : ''} · ` +
      `手册 ${playbook.length} 字 · 审计 evolution.jsonl`,
  );
}

if (process.argv[1] && process.argv[1].replace(/\\/g, '/').includes('mario-loop')) {
  void main();
}
