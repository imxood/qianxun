/**
 * 自主进化闭环(docs/13,本地侧;全程不经云端)。
 *
 *   训练局(champion,Laya 反射)
 *     → 规则复盘(postmortem.ts)
 *     → 触发器(新死因/通关/平台期才调 Qwen,同签名去重,runId 幂等)
 *     → Qwen 提案(新版手册 + 沙箱内策略补丁)
 *     → sanitize → 限幅(≤3 字段、±20%/±8px)→ 冷却期过滤 → **候选**
 *   候选须经 K 局试跑中位数对比 champion:过 +ε 才转正(commit),
 *   否则回滚 + 被否决字段冷却 M 局。审计 evolution.jsonl 只追加。
 *
 * 成功概率↑ / 耗时↓ 由 history.json 逐局曲线度量;云端编码代理只在
 * 需要改代码/引擎本身时介入,参数级优化全在这个循环内完成。
 *
 * 用法:npx tsx e2e/mario-loop.ts [局数=5] [--mode=reflex|autopilot]
 *       [--dir=<状态目录>] [--eval-runs=3]
 * 默认状态目录:~/.qianxun_dev/mario(与 UI 共享 champion);旧 var/mario 自动迁移。
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { MarioDriver, type DriverStats } from '../src/features/games/mario/driver';
import { createGameState, step, type GameState } from '../src/features/games/mario/engine';
import {
  EVAL_RUNS_DEFAULT,
  EVOLUTION_FILE,
  HISTORY_FILE,
  PLAYBOOK_FILE,
  POLICY_FILE,
  STATE_FILE,
  applyCooldowns,
  appendHistory,
  auditLine,
  diffPatch,
  evalMedian,
  judgeCandidate,
  limitPatch,
  parseEvoState,
  parseHistory,
  parsePolicyFile,
  recordRun,
  shouldInvokeQwen,
  startCooldowns,
  wrapPolicy,
  type AuditEntry,
  type EvoState,
  type PolicyEnvelope,
  type RunOutcome,
} from '../src/features/games/mario/evolution';
import type { PostmortemReport } from '../src/features/games/mario/postmortem';
import { getPolicy, setPolicy, type PolicyProfile } from '../src/features/games/mario/policy';
import {
  PLAYBOOK_MAX_CHARS,
  refineViaQwen,
  type HistoryRow,
} from '../src/features/games/mario/qwen';
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
  /** Qwen 触发器判定(未调用的原因 / 调用的理由)。 */
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

export function loadState(dir: string): LoopState {
  const { envelope } = parsePolicyFile(readText(dir, POLICY_FILE));
  const playbook = readText(dir, PLAYBOOK_FILE) ?? '';
  // 历史:history.json 优先;旧 evolution.json 兼容读
  let history = parseHistory(readText(dir, HISTORY_FILE));
  if (history.length === 0) history = parseHistory(readText(dir, 'evolution.json'));
  const { state: evo } = parseEvoState(readText(dir, STATE_FILE));
  setPolicy(envelope.policy);
  return { envelope, playbook, history, evo };
}

export function saveState(dir: string, state: LoopState): void {
  fs.mkdirSync(dir, { recursive: true });
  const envelope: PolicyEnvelope = { ...state.envelope, updatedAt: new Date().toISOString() };
  fs.writeFileSync(path.join(dir, POLICY_FILE), JSON.stringify(envelope, null, 2));
  fs.writeFileSync(path.join(dir, PLAYBOOK_FILE), state.playbook.slice(0, PLAYBOOK_MAX_CHARS));
  fs.writeFileSync(path.join(dir, HISTORY_FILE), JSON.stringify(state.history, null, 2));
  fs.writeFileSync(path.join(dir, STATE_FILE), JSON.stringify(state.evo, null, 2));
}

/** 审计只追加:每条规则变更的完整证据链(docs/13 §5)。 */
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
  return { won, ticks: s.tick, stats: { ...driver.stats }, report, runId: driver.runId };
}

/** 局后分析(委托共享客户端 qwen.ts;保留旧签名供测试/脚本)。 */
export async function refineWithQwen(input: {
  playbook: string;
  report: PostmortemReport;
  policy: PolicyProfile;
  history: HistoryRow[];
  endpoint?: string;
  fetchImpl?: typeof fetch;
}): Promise<{ playbook: string; policy: PolicyProfile; issues: string[] }> {
  return refineViaQwen(input);
}

// ---------- 主循环:候选—评估—提交(docs/13 §4) ----------

type PendingCandidate = {
  policy: PolicyProfile;
  patch: Record<string, { from: unknown; to: unknown }>;
  issues: string[];
  reason: string;
};

export async function runLoop(opts: {
  iterations: number;
  mode: 'reflex' | 'autopilot';
  dataDir: string;
  /** 候选试跑局数(中位数聚合),默认 EVAL_RUNS_DEFAULT。 */
  evalRuns?: number;
  log?: (line: string) => void;
}): Promise<IterationResult[]> {
  const log = opts.log ?? ((line: string) => console.log(line));
  const K = Math.max(1, opts.evalRuns ?? EVAL_RUNS_DEFAULT);
  const state = loadState(opts.dataDir);
  const results: IterationResult[] = [];
  let pending: PendingCandidate | null = null;

  const evalK = async (policy: PolicyProfile): Promise<RunOutcome[]> => {
    const runs: RunOutcome[] = [];
    for (let j = 0; j < K; j += 1) {
      const r = await runHeadlessSession({ mode: opts.mode, maxTicks: MAX_TICKS, policy });
      runs.push({ won: r.won, maxXCol: r.report.maxXCol, ticks: r.ticks });
    }
    return runs;
  };

  for (let i = 1; i <= opts.iterations; i += 1) {
    const t0 = Date.now();
    let verdict: IterationResult['verdict'];
    let triggerReason = 'autopilot(教师采集,不调 Qwen)';
    let qwenInvoked = false;
    let policyIssues: string[] = [];
    try {
      // ① 评估上一轮候选:测量通过才转正(LLM 负责创意,代码负责纪律)
      if (pending !== null) {
        const iter = state.evo.iteration;
        if (state.evo.championScore === null) {
          const base = await evalK(state.envelope.policy);
          state.evo.championScore = evalMedian(base);
          appendAudit(opts.dataDir, {
            v: 1,
            ts: new Date().toISOString(),
            iter,
            runId: null,
            actor: 'system',
            action: 'evaluate',
            reason: `champion 基线测量(K=${K})`,
            score: { champion: state.evo.championScore, candidate: null },
          });
          log(`[局 ${i}] champion 基线:中位分 ${state.evo.championScore.toFixed(1)}(K=${K})`);
        }
        const cRuns = await evalK(pending.policy);
        const cScore = evalMedian(cRuns);
        const v = judgeCandidate(state.evo.championScore, cScore);
        verdict = v.verdict;
        if (v.verdict === 'commit') {
          state.envelope = wrapPolicy(pending.policy, `commit:iter-${iter}`);
          state.evo.championScore = cScore;
          setPolicy(pending.policy);
        } else {
          state.evo.cooldowns = startCooldowns(
            state.evo.cooldowns,
            Object.keys(pending.patch),
            iter,
          );
        }
        appendAudit(opts.dataDir, {
          v: 1,
          ts: new Date().toISOString(),
          iter,
          runId: null,
          actor: 'system',
          action: v.verdict,
          reason: v.reason,
          patch: pending.patch,
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
      const run = await runHeadlessSession({
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

      // ③ 触发器:有新信息才调 Qwen
      if (opts.mode === 'reflex') {
        const trigger = shouldInvokeQwen(state.evo, run.report, run.runId);
        triggerReason = trigger.reason;
        if (trigger.invoke) {
          qwenInvoked = true;
          const refined = await refineWithQwen({
            playbook: state.playbook,
            report: run.report,
            policy: champion,
            history: state.history,
          });
          state.playbook = refined.playbook; // 手册立即沉淀(教训不回滚)
          const limited = limitPatch(refined.policy, champion);
          const cooled = applyCooldowns(
            limited.policy,
            champion,
            state.evo.cooldowns,
            state.evo.iteration + 1,
          );
          const patch = diffPatch(champion, cooled.policy);
          policyIssues = [...refined.issues, ...limited.issues];
          if (Object.keys(patch).length > 0) {
            pending = {
              policy: cooled.policy,
              patch,
              issues: policyIssues,
              reason: trigger.reason,
            };
            appendAudit(opts.dataDir, {
              v: 1,
              ts: new Date().toISOString(),
              iter: state.evo.iteration + 1,
              runId: run.runId,
              actor: 'qwen',
              action: 'propose',
              reason: trigger.reason,
              patch,
              issues: policyIssues,
            });
            log(
              `[局 ${i}] Qwen 提案 ${Object.keys(patch).length} 字段(候选,待 K=${K} 试跑) ` +
                `· 手册 ${state.playbook.length} 字`,
            );
          } else {
            appendAudit(opts.dataDir, {
              v: 1,
              ts: new Date().toISOString(),
              iter: state.evo.iteration + 1,
              runId: run.runId,
              actor: 'qwen',
              action: 'skip',
              reason: '补丁为空或与 champion 无差异',
              issues: policyIssues,
            });
            log(`[局 ${i}] Qwen 补丁无有效变更,丢弃(手册仍更新)`);
          }
        } else {
          appendAudit(opts.dataDir, {
            v: 1,
            ts: new Date().toISOString(),
            iter: state.evo.iteration + 1,
            runId: run.runId,
            actor: 'system',
            action: 'skip',
            reason: trigger.reason,
          });
          log(`[局 ${i}] 不调 Qwen:${trigger.reason}`);
        }
      }

      state.evo = recordRun(state.evo, run.report, run.runId, qwenInvoked);
      saveState(opts.dataDir, state);
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
