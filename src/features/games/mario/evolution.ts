/**
 * 进化闭环的纯逻辑核心(docs/13):状态形状、policy 信封、Qwen 触发器、
 * 评估判定、冷却期、补丁限幅。全部纯函数——UI(MarioLab)与无头闭环
 * (e2e/mario-loop.ts)共用同一份规则,存储读写分离在 store.ts / 闭环自身。
 *
 * 三条不变量的代码化:
 * ① 局内配置不可变 —— 见 driver.beginRun 快照;
 * ② 规则复盘在前 —— 触发器只消费 PostmortemReport,不看原始遥测;
 * ③ 补丁是候选 —— judgeCandidate 测量通过才提交,"LLM 负责创意,代码负责纪律"。
 */

import {
  DEFAULT_POLICY,
  FIELD_SPECS,
  sanitizePolicy,
  type FieldDim,
  type PolicyProfile,
} from './policy';
import type { PostmortemReport } from './postmortem';
import type { HistoryRow } from './qwen';

// ---- 状态文件名(存储布局,docs/13 §5) ----
export const POLICY_FILE = 'policy.json';
export const PLAYBOOK_FILE = 'playbook.md';
export const HISTORY_FILE = 'history.json';
export const STATE_FILE = 'state.json';
export const EVOLUTION_FILE = 'evolution.jsonl';
export const INSIGHTS_FILE = 'insights.md';

// ---- 协议常数(docs/13 §4 + docs/15,可调) ----
export const EVAL_RUNS_DEFAULT = 3; // 候选试跑局数(取中位数)
export const DESIGN_EVAL_RUNS = 5; // 消息设计变更评估窗(输入漂移需更长观测)
export const EPSILON = 0.05; // 提交最小提升阈值
export const COOLDOWN_ITERS = 5; // 字段回滚后的冷却迭代数
export const PLATEAU_WINDOW = 3; // 连续无进展局数触发求助
export const MAX_PATCH_FIELDS = 3; // 单次补丁最多字段数
export const MAX_NUMERIC_STEP = 0.2; // 数值字段单次步进 ≤20%
export const MAX_WINDOW_STEP_PX = 8; // 触发窗端点单次移动 ≤8px
const ANALYZED_CAP = 50;
const DEATHKEY_CAP = 200; // 死亡级幂等键容量(≈66 局)
const SIGSTATS_CAP = 50; // 签名统计容量(滚动)
const LEDGER_CAP = 20; // 变更-效果账本容量
const VERSIONS_CAP = 20; // policy 版本快照容量
const HISTORY_CAP = 200; // 趋势与北极星窗口需要(50 局 ≈1h 就滚没了)

// ================= policy 信封(版本化持久化) =================

export type PolicyEnvelope = {
  version: 1;
  updatedAt: string;
  /** 来源:'default' | 'legacy' | 'user' | 'qwen:iter-N' | 'commit:iter-N' */
  source: string;
  policy: PolicyProfile;
};

export function wrapPolicy(
  policy: PolicyProfile,
  source: string,
  updatedAt = new Date().toISOString(),
): PolicyEnvelope {
  return { version: 1, updatedAt, source, policy };
}

/**
 * 解析持久化的 policy.json:信封 v1 / 旧版扁平 JSON 自动迁移 / 垃圾回退默认。
 * 加载即 sanitize(docs/13 §2.3):不信任任何外部来源,包括昨天自己写下的文件。
 */
export function parsePolicyFile(raw: string | null): {
  envelope: PolicyEnvelope;
  issues: string[];
} {
  if (raw === null || raw.trim() === '') {
    return { envelope: wrapPolicy(DEFAULT_POLICY, 'default'), issues: [] };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {
      envelope: wrapPolicy(DEFAULT_POLICY, 'default'),
      issues: ['policy.json 不是合法 JSON,已回退默认策略'],
    };
  }
  if (typeof parsed !== 'object' || parsed === null) {
    return {
      envelope: wrapPolicy(DEFAULT_POLICY, 'default'),
      issues: ['policy.json 顶层不是对象,已回退默认策略'],
    };
  }
  const rec = parsed as Record<string, unknown>;
  // 旧版扁平格式(顶层即 PolicyProfile)→ 迁移为信封
  if (typeof rec['intervalMs'] === 'number' && rec['policy'] === undefined) {
    const s = sanitizePolicy(rec);
    return {
      envelope: wrapPolicy(s.policy, 'legacy'),
      issues: s.issues,
    };
  }
  if (rec['version'] !== 1 || typeof rec['policy'] !== 'object' || rec['policy'] === null) {
    return {
      envelope: wrapPolicy(DEFAULT_POLICY, 'default'),
      issues: ['policy.json 版本无法识别,已回退默认策略'],
    };
  }
  const s = sanitizePolicy(rec['policy']);
  return {
    envelope: {
      version: 1,
      updatedAt: typeof rec['updatedAt'] === 'string' ? rec['updatedAt'] : '',
      source: typeof rec['source'] === 'string' ? rec['source'] : 'unknown',
      policy: s.policy,
    },
    issues: s.issues,
  };
}

// ================= 进化簿记状态(state.json,可从审计重建) =================

/** Qwen 分析计数(docs/15 §2):失败必须一眼可见,启动时与审计重放对账。 */
export type QwenCounters = {
  deathAnalyses: number;
  runAnalyses: number;
  failed: number;
  timeouts: number;
  badJson: number;
  overLength: number;
  retries: number;
  merged: number;
  droppedByCap: number;
  queueHighWater: number;
  emptyInsight: number;
  patches: number;
  commits: number;
  rollbacks: number;
};

export type Counters = {
  runs: number;
  wins: number;
  deaths: number;
  deathsByCause: Record<string, number>;
  deathsByLandmark: Record<string, number>;
  qwen: QwenCounters;
  perMode: Record<string, { runs: number; wins: number }>;
};

/** 签名统计(docs/15 §6.7 升级阶梯):deaths≥2 即 repeat;proposals 驱动换维度。 */
export type SigStat = { deaths: number; proposals: number; lastImprovedIter: number };

/** 落盘的分析任务(队列溢出 backlog,docs/15 §6.2:不丢死亡)。 */
export type PersistTask = {
  key: string;
  kind: 'death' | 'win' | 'plateau';
  attempt?: number;
  sig?: string | null;
  reason: string;
  ts: string;
};

/** 变更-效果账本行(docs/15 §6.7 D1):信用分配的地基。 */
export type ChangeLedgerRow = {
  changeId: string;
  vid: number;
  fields: Record<string, { from: unknown; to: unknown }>;
  kind: 'numeric' | 'design';
  dimension: FieldDim;
  originSigs: string[];
  deathKeys: string[];
  verdict: 'commit' | 'rollback';
  scoreDelta: number | null;
  sigDelta?: Record<string, { repeatBefore: number; repeatAfter: number }>;
};

/** policy 版本快照(docs/15 §7 D6):回退到第 N 好。 */
export type VersionRow = {
  vid: number;
  iter: number;
  ts: string;
  policy: PolicyProfile;
  patch: Record<string, { from: unknown; to: unknown }>;
  originSigs: string[];
  verdict: 'commit' | 'rollback';
  score: number;
};

export type EvoState = {
  /** 已完成训练局数(Qwen 提案周期的计数) */
  iteration: number;
  /** champion 最近一次评估中位分;null = 尚未测量(仅 iteration=0 允许直转) */
  championScore: number | null;
  /** 字段 → 冷却截止迭代号(含);回滚过的字段冷却期内禁止再改 */
  cooldowns: Record<string, number>;
  /** 已做过 Qwen 分析的 runId,run 级(win/plateau)幂等(滚动) */
  analyzedRunIds: string[];
  /** 上次分析过的失败签名(死因@地标);null = 尚无 */
  lastDeathSignature: string | null;
  bestMaxXCol: number;
  plateauStreak: number;
  // ---- docs/15 v3 新增(schemaVersion 兼容:旧文件读入补默认) ----
  /** 死亡级幂等键 `runId#attempt`(成功后才占,滚动 200) */
  analyzedDeathKeys: string[];
  /** 队列溢出落盘 backlog(内存队列 cap 满时不丢死亡) */
  pendingAnalyses: PersistTask[];
  /** 签名统计(升级阶梯/重复判定) */
  sigStats: Record<string, SigStat>;
  /** 统计计数器(§2;审计为原始事实,可重放对账) */
  counters: Counters;
  /** 当前 champion 版本号 */
  championVid: number;
  /** 版本快照账本(cap 20) */
  versions: VersionRow[];
  /** 变更-效果账本(cap 20) */
  ledger: ChangeLedgerRow[];
};

export function initialCounters(): Counters {
  return {
    runs: 0,
    wins: 0,
    deaths: 0,
    deathsByCause: {},
    deathsByLandmark: {},
    qwen: {
      deathAnalyses: 0,
      runAnalyses: 0,
      failed: 0,
      timeouts: 0,
      badJson: 0,
      overLength: 0,
      retries: 0,
      merged: 0,
      droppedByCap: 0,
      queueHighWater: 0,
      emptyInsight: 0,
      patches: 0,
      commits: 0,
      rollbacks: 0,
    },
    perMode: {},
  };
}

export function initialEvoState(): EvoState {
  return {
    iteration: 0,
    championScore: null,
    cooldowns: {},
    analyzedRunIds: [],
    lastDeathSignature: null,
    bestMaxXCol: 0,
    plateauStreak: 0,
    analyzedDeathKeys: [],
    pendingAnalyses: [],
    sigStats: {},
    counters: initialCounters(),
    championVid: 0,
    versions: [],
    ledger: [],
  };
}

function numOr(v: unknown, d: number): number {
  return typeof v === 'number' ? v : d;
}
function strArr(v: unknown, cap: number): string[] {
  return Array.isArray(v) ? (v as string[]).filter((x) => typeof x === 'string').slice(-cap) : [];
}

export function parseEvoState(raw: string | null): { state: EvoState; issues: string[] } {
  if (raw === null || raw.trim() === '') return { state: initialEvoState(), issues: [] };
  try {
    const rec = JSON.parse(raw) as Record<string, unknown>;
    const cRec = (
      typeof rec['counters'] === 'object' && rec['counters'] !== null ? rec['counters'] : {}
    ) as Record<string, unknown>;
    const qRec = (
      typeof cRec['qwen'] === 'object' && cRec['qwen'] !== null ? cRec['qwen'] : {}
    ) as Record<string, unknown>;
    const counters = initialCounters();
    counters.runs = numOr(cRec['runs'], 0);
    counters.wins = numOr(cRec['wins'], 0);
    counters.deaths = numOr(cRec['deaths'], 0);
    counters.deathsByCause =
      typeof cRec['deathsByCause'] === 'object' && cRec['deathsByCause'] !== null
        ? (cRec['deathsByCause'] as Record<string, number>)
        : {};
    counters.deathsByLandmark =
      typeof cRec['deathsByLandmark'] === 'object' && cRec['deathsByLandmark'] !== null
        ? (cRec['deathsByLandmark'] as Record<string, number>)
        : {};
    for (const k of Object.keys(counters.qwen) as (keyof QwenCounters)[]) {
      counters.qwen[k] = numOr(qRec[k], 0);
    }
    counters.perMode =
      typeof cRec['perMode'] === 'object' && cRec['perMode'] !== null
        ? (cRec['perMode'] as Counters['perMode'])
        : {};
    return {
      state: {
        iteration: numOr(rec['iteration'], 0),
        championScore: typeof rec['championScore'] === 'number' ? rec['championScore'] : null,
        cooldowns:
          typeof rec['cooldowns'] === 'object' && rec['cooldowns'] !== null
            ? (rec['cooldowns'] as Record<string, number>)
            : {},
        analyzedRunIds: strArr(rec['analyzedRunIds'], ANALYZED_CAP),
        lastDeathSignature:
          typeof rec['lastDeathSignature'] === 'string' ? rec['lastDeathSignature'] : null,
        bestMaxXCol: numOr(rec['bestMaxXCol'], 0),
        plateauStreak: numOr(rec['plateauStreak'], 0),
        analyzedDeathKeys: strArr(rec['analyzedDeathKeys'], DEATHKEY_CAP),
        pendingAnalyses: Array.isArray(rec['pendingAnalyses'])
          ? (rec['pendingAnalyses'] as PersistTask[]).filter((t) => typeof t?.key === 'string')
          : [],
        sigStats:
          typeof rec['sigStats'] === 'object' && rec['sigStats'] !== null
            ? (rec['sigStats'] as Record<string, SigStat>)
            : {},
        counters,
        championVid: numOr(rec['championVid'], 0),
        versions: Array.isArray(rec['versions'])
          ? (rec['versions'] as VersionRow[]).slice(-VERSIONS_CAP)
          : [],
        ledger: Array.isArray(rec['ledger'])
          ? (rec['ledger'] as ChangeLedgerRow[]).slice(-LEDGER_CAP)
          : [],
      } satisfies EvoState,
      issues: [],
    };
  } catch {
    return { state: initialEvoState(), issues: ['state.json 损坏,簿记已重置'] };
  }
}

// ================= Qwen 触发器(docs/15 §6.1:死亡级分析清单) =================

export type AnalysisTask = {
  /** deathKey `runId#attempt` 或 run 级键 `runId#run` */
  key: string;
  kind: 'death' | 'win' | 'plateau';
  attempt?: number;
  sig: string | null;
  reason: string;
  /** first-death 0 > 新签名 1 > repeat 2 > win 3 > plateau 4(小者优先) */
  priority: number;
};

/** 死亡级签名 = 死因 @ 该次死亡地标(区别于 deathSignature 的局级聚合)。 */
function deathSigOf(d: { cause: string; landmark: string }): string {
  return `${d.cause}@${d.landmark}`;
}

/** 局级失败签名 = 最常见死因 @ 其首次发生的地标;无死亡 → null(兼容 v1)。 */
export function deathSignature(report: PostmortemReport): string | null {
  const top = report.deathCauses[0];
  if (top === undefined) return null;
  const first = report.deaths.find((d) => d.cause === top.cause);
  return `${top.cause}@${first?.landmark ?? '?'}`;
}

/**
 * docs/15 §6.1:v3 触发器返回**分析清单**——每死一个 death 级任务(一局 3 死
 * = 3 次独立分析),win/plateau 仍 run 级。幂等:death 查 analyzedDeathKeys +
 * pendingAnalyses;run 级查旧 analyzedRunIds(迁移只读)。签名重复按 sigStats
 * 计数(deaths≥2 即 repeat),不再只看相邻局。
 */
export function planAnalyses(
  state: EvoState,
  report: PostmortemReport,
  runId: string,
): AnalysisTask[] {
  const tasks: AnalysisTask[] = [];
  const queued = new Set(state.pendingAnalyses.map((t) => t.key));
  for (const d of report.deaths) {
    if (d.attempt === undefined) continue; // 旧会话无 attempt,无法构成 deathKey
    const key = `${runId}#${d.attempt}`;
    if (state.analyzedDeathKeys.includes(key) || queued.has(key)) continue;
    const sig = deathSigOf(d);
    const stat = state.sigStats[sig];
    const repeat = (stat?.deaths ?? 0) >= 1; // 本局之前已死过同签名 ≥1 次
    const reason = repeat ? `death-repeat(${sig})` : `death(${sig})`;
    const priority = d.attempt === 1 ? 0 : repeat ? 2 : 1;
    tasks.push({ key, kind: 'death', attempt: d.attempt, sig, reason, priority });
  }
  if (report.outcome === 'win') {
    if (!state.analyzedRunIds.includes(runId) && !queued.has(`${runId}#run`)) {
      tasks.push({
        key: `${runId}#run`,
        kind: 'win',
        sig: null,
        reason: 'win(沉淀成功经验)',
        priority: 3,
      });
    }
  } else if (
    report.deaths.every((d) => d.attempt === undefined) && // 旧会话无死亡级键
    state.plateauStreak >= PLATEAU_WINDOW &&
    !state.analyzedRunIds.includes(runId)
  ) {
    tasks.push({
      key: `${runId}#run`,
      kind: 'plateau',
      sig: null,
      reason: `plateau(连续 ${state.plateauStreak} 局无进展)`,
      priority: 4,
    });
  }
  return tasks.sort((a, b) => a.priority - b.priority);
}

/** 分析成功后占幂等键(修 docs/15 R6:失败不占,任务留 backlog 重试)。 */
export function markAnalyzed(state: EvoState, task: AnalysisTask): EvoState {
  if (task.kind === 'death') {
    return {
      ...state,
      analyzedDeathKeys: [...state.analyzedDeathKeys, task.key].slice(-DEATHKEY_CAP),
    };
  }
  return {
    ...state,
    analyzedRunIds: [...state.analyzedRunIds, task.key.split('#')[0]!].slice(-ANALYZED_CAP),
  };
}

/** 签名统计:死亡计数(每局每签名 +1)。 */
export function noteDeathSigs(state: EvoState, report: PostmortemReport): EvoState {
  const sigStats = { ...state.sigStats };
  for (const d of report.deaths) {
    const sig = deathSigOf(d);
    const cur = sigStats[sig] ?? { deaths: 0, proposals: 0, lastImprovedIter: 0 };
    sigStats[sig] = { ...cur, deaths: cur.deaths + 1 };
  }
  const keys = Object.keys(sigStats);
  if (keys.length <= SIGSTATS_CAP) return { ...state, sigStats };
  // 容量滚动:按 deaths 升序淘汰(冷签名先走)
  for (const k of keys
    .sort((a, b) => sigStats[a]!.deaths - sigStats[b]!.deaths)
    .slice(0, keys.length - SIGSTATS_CAP)) {
    delete sigStats[k];
  }
  return { ...state, sigStats };
}

/** 提案记账:originSigs 各 +1 proposals(升级阶梯计数)。 */
export function noteProposals(state: EvoState, sigs: string[], iter: number): EvoState {
  const sigStats = { ...state.sigStats };
  for (const sig of sigs) {
    const cur = sigStats[sig] ?? { deaths: 0, proposals: 0, lastImprovedIter: 0 };
    sigStats[sig] = { ...cur, proposals: cur.proposals + 1 };
  }
  void iter;
  return { ...state, sigStats };
}

/** 每局结束后更新簿记(无论是否调了 Qwen)。返回新状态,不原地改。 */
export function recordRun(
  state: EvoState,
  report: PostmortemReport,
  runId: string,
  analyzed: boolean,
  mode?: string,
): EvoState {
  const progressed = report.maxXCol > state.bestMaxXCol;
  const sig = deathSignature(report);
  const counters = { ...state.counters };
  counters.runs += 1;
  if (report.outcome === 'win') counters.wins += 1;
  counters.deaths += report.deaths.length;
  const byCause = { ...counters.deathsByCause };
  const byLandmark = { ...counters.deathsByLandmark };
  for (const d of report.deaths) {
    byCause[d.cause] = (byCause[d.cause] ?? 0) + 1;
    byLandmark[d.landmark] = (byLandmark[d.landmark] ?? 0) + 1;
  }
  counters.deathsByCause = byCause;
  counters.deathsByLandmark = byLandmark;
  if (mode !== undefined) {
    const m = counters.perMode[mode] ?? { runs: 0, wins: 0 };
    counters.perMode[mode] = {
      runs: m.runs + 1,
      wins: m.wins + (report.outcome === 'win' ? 1 : 0),
    };
  }
  const withSigs = noteDeathSigs({ ...state, counters }, report);
  return {
    ...withSigs,
    iteration: state.iteration + 1,
    bestMaxXCol: progressed ? report.maxXCol : state.bestMaxXCol,
    plateauStreak: progressed ? 0 : state.plateauStreak + 1,
    lastDeathSignature: sig !== null ? sig : state.lastDeathSignature,
    analyzedRunIds: analyzed
      ? [...state.analyzedRunIds, runId].slice(-ANALYZED_CAP)
      : state.analyzedRunIds,
  };
}

// ================= 评估:打分与候选判定(docs/13 §4.3) =================

export type RunOutcome = {
  won: boolean;
  maxXCol: number;
  ticks: number;
};

/**
 * 单局分数:通关 ≫ 未通关;通关局中 ticks 越小分越高("耗时越来越短");
 * 未通关按最远进度。单调可比较,中位数聚合抗噪。
 */
export function runScore(r: RunOutcome): number {
  return r.won ? 10000 - r.ticks / 10 + r.maxXCol : r.maxXCol;
}

export function median(xs: number[]): number {
  if (xs.length === 0) return 0;
  const sorted = [...xs].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  const a = sorted[mid]!;
  return sorted.length % 2 === 1 ? a : (sorted[mid - 1]! + a) / 2;
}

export function evalMedian(runs: RunOutcome[]): number {
  return median(runs.map(runScore));
}

export type Verdict = { verdict: 'commit' | 'rollback'; reason: string };

/**
 * 滞回判定(docs/13 §4.2-4):只有超过 +ε 才替换 champion;
 * 未过阈值一律回滚候选——涨跌不对称,噪声被吸收。
 */
export function judgeCandidate(
  championScore: number | null,
  candidateScore: number,
  epsilon = EPSILON,
): Verdict {
  if (championScore === null) {
    return { verdict: 'commit', reason: '无 champion 基线,首个候选直接转正' };
  }
  const threshold = championScore * (1 + epsilon);
  if (candidateScore > threshold) {
    return {
      verdict: 'commit',
      reason: `候选中位分 ${candidateScore.toFixed(1)} > champion ${championScore.toFixed(1)}×(1+${epsilon})=${threshold.toFixed(1)}`,
    };
  }
  return {
    verdict: 'rollback',
    reason: `候选中位分 ${candidateScore.toFixed(1)} 未过提交阈值 ${threshold.toFixed(1)}(+ε 滞回)`,
  };
}

// ================= 补丁纪律:差异、限幅、冷却期 =================

/** 两策略的变更字段集(audit / 空补丁检测用)。 */
export function diffPatch(
  base: PolicyProfile,
  target: PolicyProfile,
): Record<string, { from: unknown; to: unknown }> {
  const out: Record<string, { from: unknown; to: unknown }> = {};
  for (const key of Object.keys(DEFAULT_POLICY) as (keyof PolicyProfile)[]) {
    const a = base[key];
    const b = target[key];
    const same = Array.isArray(a) && Array.isArray(b) ? a[0] === b[0] && a[1] === b[1] : a === b;
    if (!same) out[key] = { from: a, to: b };
  }
  return out;
}

/**
 * 变更幅度限制(docs/13 §4.2-2 + docs/15 §6.4/6.7):
 * - **单维度原则(代码强制)**:一次补丁字段必须同属一个 FIELD_SPECS 维度;
 *   数值调参与消息设计变体不得同车(混窗不可归因)。跨维度时保留字段最多的
 *   维度,其余丢弃并写 issue。
 * - 最多保留 MAX_PATCH_FIELDS 个字段(按相对变化幅度从大到小);
 * - 数值标量步进钳到 ±20%;触发窗端点各钳 ±8px;布尔/枚举/清单直接放行。
 * 返回 kind('numeric'|'design')供审计 patchKind 与评估窗选择(3 vs 5 局)。
 */
export function limitPatch(
  patch: PolicyProfile,
  current: PolicyProfile,
): { policy: PolicyProfile; issues: string[]; kind: 'numeric' | 'design' | 'none' } {
  const issues: string[] = [];
  const out: PolicyProfile = { ...current };
  type Chg = { key: keyof PolicyProfile; magnitude: number; dim: FieldDim };
  const changed: Chg[] = [];
  for (const key of Object.keys(DEFAULT_POLICY) as (keyof PolicyProfile)[]) {
    const spec = FIELD_SPECS[key as string];
    if (!spec) continue; // 声明表之外的字段不进补丁(静态性声明,docs/15 §6.6)
    const a = current[key];
    const b = patch[key];
    const dim = spec.dim;
    if (typeof a === 'boolean' && typeof b === 'boolean') {
      if (a !== b) changed.push({ key, magnitude: 1, dim });
      continue;
    }
    if (typeof a === 'number' && typeof b === 'number') {
      const rel = Math.abs(b - a) / Math.max(1, Math.abs(a));
      if (rel > 0.001) changed.push({ key, magnitude: rel, dim });
      continue;
    }
    if (Array.isArray(a) && Array.isArray(b)) {
      if (typeof b[0] === 'number' && typeof a[0] === 'number') {
        // 触发窗:端点距离折算幅度
        if (b[0] !== a[0] || b[1] !== a[1]) {
          const aw = a as unknown as [number, number];
          const bw = b as unknown as [number, number];
          changed.push({
            key,
            magnitude: Math.max(Math.abs(bw[0] - aw[0]), Math.abs(bw[1] - aw[1])) / 16,
            dim,
          });
        }
      } else if ((a as unknown[]).join(',') !== (b as unknown[]).join(',')) {
        changed.push({ key, magnitude: 1, dim }); // extra-list(string[])
      }
      continue;
    }
    // 枚举变更:登记为幅度 1
    if (a !== b) changed.push({ key, magnitude: 1, dim });
  }
  if (changed.length === 0) return { policy: out, issues, kind: 'none' };
  // 单维度:按字段数保留一个维度(平局取幅度和更大者)
  const groups = new Map<FieldDim, Chg[]>();
  for (const c of changed) {
    const g = groups.get(c.dim) ?? [];
    g.push(c);
    groups.set(c.dim, g);
  }
  let keepDim: FieldDim = changed[0]!.dim;
  let keepLen = -1;
  let keepSum = -1;
  for (const [dim, g] of groups) {
    const sum = g.reduce((s, c) => s + c.magnitude, 0);
    if (g.length > keepLen || (g.length === keepLen && sum > keepSum)) {
      keepDim = dim;
      keepLen = g.length;
      keepSum = sum;
    }
  }
  const kept = [...(groups.get(keepDim) ?? [])].sort((x, y) => y.magnitude - x.magnitude);
  for (const c of changed) {
    if (c.dim !== keepDim) {
      issues.push(`限幅:${c.key} 维度 ${c.dim} 与 ${keepDim} 混车,已丢弃(单变更原则)`);
    }
  }
  for (const [i, c] of kept.entries()) {
    if (i >= MAX_PATCH_FIELDS) {
      issues.push(`限幅:字段 ${c.key} 超出单次最多 ${MAX_PATCH_FIELDS} 个,已丢弃`);
      continue;
    }
    const a = current[c.key];
    const b = patch[c.key];
    if (typeof a === 'number' && typeof b === 'number') {
      const maxDelta = Math.max(1, Math.abs(a) * MAX_NUMERIC_STEP);
      const clamped = a + Math.max(-maxDelta, Math.min(maxDelta, b - a));
      if (clamped !== b) {
        issues.push(`限幅:${c.key} ${a}→${b} 步进超 ±${MAX_NUMERIC_STEP * 100}%,钳到 ${clamped}`);
      }
      (out as Record<string, unknown>)[c.key] = clamped;
    } else if (
      Array.isArray(a) &&
      Array.isArray(b) &&
      typeof b[0] === 'number' &&
      typeof a[0] === 'number'
    ) {
      const aw = a as unknown as [number, number];
      const bw = b as unknown as [number, number];
      const clamp = (from: number, to: number): number =>
        from + Math.max(-MAX_WINDOW_STEP_PX, Math.min(MAX_WINDOW_STEP_PX, to - from));
      const w: [number, number] = [clamp(aw[0], bw[0]), clamp(aw[1], bw[1])];
      if (w[0] !== b[0] || w[1] !== b[1]) {
        issues.push(`限幅:${c.key} 端点移动超 ±${MAX_WINDOW_STEP_PX}px,钳到 [${w[0]},${w[1]}]`);
      }
      (out as Record<string, unknown>)[c.key] = w;
    } else {
      (out as Record<string, unknown>)[c.key] = b; // 布尔/枚举/清单直改
    }
  }
  return { policy: out, issues, kind: keepDim === 'message-design' ? 'design' : 'numeric' };
}

/** 冷却期过滤:迭代号 ≤ cooldowns[field] 的字段从补丁中剔除。 */
export function applyCooldowns(
  patch: PolicyProfile,
  current: PolicyProfile,
  cooldowns: Record<string, number>,
  iteration: number,
): { policy: PolicyProfile; dropped: string[] } {
  const out: PolicyProfile = { ...patch };
  const dropped: string[] = [];
  for (const key of Object.keys(diffPatch(current, patch))) {
    const until = cooldowns[key];
    if (until !== undefined && iteration <= until) {
      (out as Record<string, unknown>)[key] = (current as Record<string, unknown>)[key];
      dropped.push(key);
    }
  }
  return { policy: out, dropped };
}

/** 回滚后为被否决字段开启冷却期。 */
export function startCooldowns(
  cooldowns: Record<string, number>,
  fields: string[],
  iteration: number,
  span = COOLDOWN_ITERS,
): Record<string, number> {
  const out = { ...cooldowns };
  for (const f of fields) out[f] = iteration + span;
  return out;
}

// ================= 审计(evolution.jsonl,只追加) =================

export type AuditEntry = {
  v: 1 | 2;
  ts: string;
  iter: number;
  runId: string | null;
  actor: 'qwen' | 'user' | 'system';
  action:
    | 'evaluate'
    | 'propose'
    | 'propose-stale'
    | 'trial'
    | 'commit'
    | 'rollback'
    | 'skip'
    | 'analyze'
    | 'restore';
  reason: string;
  patch?: Record<string, { from: unknown; to: unknown }>;
  score?: { champion: number | null; candidate: number | null };
  /** 观测模式快照(docs/14 §4.3 + docs/15 §6.4 消息设计):消融可追溯。 */
  obs?: ObsSnapshot;
  issues?: string[];
  /** analyze 行专用:phase/key/错误分类/耗时 */
  phase?: 'start' | 'ok' | 'fail';
  key?: string;
  errClass?: string;
  latencyMs?: number;
  /** 补丁维度分类(docs/15 §6.4):numeric 与 design 禁混窗 */
  patchKind?: 'numeric' | 'design';
};

/** 观测模式快照:obs* 数值字段 + 消息设计选型。 */
export type ObsSnapshot = {
  profileCols: number;
  threatsLookPx: number;
  pose: boolean;
  subgoal: boolean;
  threatFormat: string;
  progressStyle: string;
  hintStyle: string;
  instructionVariant: string;
  stateExtra: string;
};

export function obsSnapshotOf(p: PolicyProfile): ObsSnapshot {
  return {
    profileCols: p.obsProfileCols,
    threatsLookPx: p.obsThreatsLookPx,
    pose: p.obsIncludePose,
    subgoal: p.obsIncludeSubgoal,
    threatFormat: p.obsThreatFormat,
    progressStyle: p.obsProgressStyle,
    hintStyle: p.obsHintStyle,
    instructionVariant: p.obsInstructionVariant,
    stateExtra: p.obsStateExtra.join(','),
  };
}

/** 审计 reason → UI 短语(两层文案不互渗:JSONL 保留原始枚举,docs/15 §5.6)。 */
export function reasonShort(reason: string): string {
  if (reason.startsWith('already-analyzed')) return '已析过';
  if (reason.startsWith('no-signal')) return '无信号';
  if (reason.startsWith('plateau')) return `平台期${reason.match(/\d+/)?.[0] ?? '?'}局`;
  if (reason.startsWith('death-repeat'))
    return `重复死亡:${reason.slice('death-repeat('.length, -1)}`;
  if (reason.startsWith('death')) return `死亡:${reason.slice('death('.length, -1)}`;
  if (reason.startsWith('win')) return '通关沉淀';
  return reason;
}

export function auditLine(e: AuditEntry): string {
  return JSON.stringify(e);
}

export function parseAuditLines(raw: string | null): AuditEntry[] {
  if (raw === null) return [];
  const out: AuditEntry[] = [];
  for (const line of raw.split('\n')) {
    const t = line.trim();
    if (t === '') continue;
    try {
      const e = JSON.parse(t) as AuditEntry;
      // 版本白名单:升版不静默丢行(docs/15 §7)
      if ((e.v === 1 || e.v === 2) && typeof e.action === 'string') out.push(e);
    } catch {
      // 坏行跳过:审计日志宁可缺行不可崩
    }
  }
  return out;
}

// ================= 经验沉淀(insights.md,docs/14 §6 + docs/15 §7) =================

// 必填协议下数小时滚动一轮,2400 会把早期基础结论冲掉(analysis-15-4)
export const INSIGHTS_MAX_CHARS = 4800;
const HUMAN_OPEN = '<!-- HUMAN -->';
const HUMAN_CLOSE = '<!-- /HUMAN -->';

/** 拆出人保段(标记对之间原样保留)与可滚动正文。 */
function splitHuman(doc: string): { humans: string[]; body: string } {
  const humans: string[] = [];
  const bodyParts: string[] = [];
  let rest = doc;
  for (;;) {
    const a = rest.indexOf(HUMAN_OPEN);
    const b = a < 0 ? -1 : rest.indexOf(HUMAN_CLOSE, a);
    if (a < 0 || b < 0) {
      bodyParts.push(rest);
      break;
    }
    bodyParts.push(rest.slice(0, a));
    humans.push(rest.slice(a, b + HUMAN_CLOSE.length));
    rest = rest.slice(b + HUMAN_CLOSE.length);
  }
  return { humans, body: bodyParts.join('\n') };
}

/**
 * 追加一条 Laya 使用经验并滚动到上限(docs/14 §6):
 * 人保段(`<!-- HUMAN -->` 标记对)永不丢,其余按行 FIFO 丢弃最旧。
 * insight 正文压成单行(≤200 字在 qwen.parseRefine 已钳制)。
 */
export function appendInsight(doc: string, iter: number, text: string, obs?: ObsSnapshot): string {
  const clean = text.trim().replace(/\s+/g, ' ');
  if (clean === '') return doc;
  const { humans, body } = splitHuman(doc);
  const lines = body.split('\n').filter((l) => l.trim() !== '');
  const obsTag = obs
    ? ` (obs ${obs.profileCols}列/${obs.threatsLookPx}px/pose:${obs.pose ? 'on' : 'off'}/sub:${obs.subgoal ? 'on' : 'off'})`
    : '';
  lines.push(`- [iter ${iter}] ${clean}${obsTag}`);
  let out = lines.join('\n');
  while (out.length > INSIGHTS_MAX_CHARS && lines.length > 1) {
    lines.shift();
    out = lines.join('\n');
  }
  return [...humans, out].filter((p) => p.trim() !== '').join('\n\n');
}

// ================= 历史(history.json) =================

export function parseHistory(raw: string | null): HistoryRow[] {
  if (raw === null) return [];
  try {
    const arr = JSON.parse(raw) as unknown;
    if (!Array.isArray(arr)) return [];
    return arr
      .filter(
        (r): r is HistoryRow =>
          typeof r === 'object' &&
          r !== null &&
          typeof (r as HistoryRow).iteration === 'number' &&
          typeof (r as HistoryRow).maxXCol === 'number',
      )
      .slice(-HISTORY_CAP);
  } catch {
    return [];
  }
}

export function appendHistory(history: HistoryRow[], row: HistoryRow): HistoryRow[] {
  return [...history, row].slice(-HISTORY_CAP);
}

// ================= 版本账本与回归守卫(docs/15 §6.7/§7) =================

/** 记录一个 policy 版本(cap 20,vid 单调递增)。 */
export function recordVersion(
  state: EvoState,
  row: Omit<VersionRow, 'vid' | 'iter' | 'ts'> & { iter?: number; ts?: string },
): EvoState {
  const vid = state.versions.reduce((m, v) => Math.max(m, v.vid), state.championVid) + 1;
  const versions: VersionRow[] = [
    ...state.versions,
    {
      vid,
      iter: row.iter ?? state.iteration,
      ts: row.ts ?? new Date().toISOString(),
      policy: row.policy,
      patch: row.patch,
      originSigs: row.originSigs,
      verdict: row.verdict,
      score: row.score,
    },
  ].slice(-VERSIONS_CAP);
  return { ...state, versions, championVid: row.verdict === 'commit' ? vid : state.championVid };
}

/** 记录账本行(cap 20)。 */
export function recordLedger(state: EvoState, row: ChangeLedgerRow): EvoState {
  return { ...state, ledger: [...state.ledger, row].slice(-LEDGER_CAP) };
}

/** 回退到指定版本:champion 换届、基线清零(强制重测)、审计由调用方落。 */
export function restoreVersion(
  state: EvoState,
  vid: number,
): { state: EvoState; policy: PolicyProfile } | null {
  const v = state.versions.find((x) => x.vid === vid);
  if (!v) return null;
  return {
    state: { ...state, championVid: vid, championScore: null, cooldowns: {} },
    policy: v.policy,
  };
}

/** 滚动 10 局通关率(北极星,docs/15 §7)。 */
export function winRate10(history: HistoryRow[]): number {
  const w = history.slice(-10);
  if (w.length === 0) return 0;
  return w.filter((r) => r.won).length / w.length;
}

/**
 * 退化检测:近 10 局通关率 < 历史最优窗口的 50% → 回归。
 * 返回 null 表示样本不足或无回归。
 */
export function detectRegression(history: HistoryRow[]): { current: number; best: number } | null {
  if (history.length < 20) return null;
  const current = winRate10(history.slice(-10));
  let best = 0;
  for (let i = 0; i + 10 <= history.length; i += 5) {
    const w = history.slice(i, i + 10);
    const rate = w.filter((r) => r.won).length / Math.max(1, w.length);
    best = Math.max(best, rate);
  }
  if (best <= 0) return null;
  if (current < best * 0.5) return { current, best };
  return null;
}
