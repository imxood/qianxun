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

import { DEFAULT_POLICY, sanitizePolicy, type PolicyProfile } from './policy';
import type { PostmortemReport } from './postmortem';
import type { HistoryRow } from './qwen';

// ---- 状态文件名(存储布局,docs/13 §5) ----
export const POLICY_FILE = 'policy.json';
export const PLAYBOOK_FILE = 'playbook.md';
export const HISTORY_FILE = 'history.json';
export const STATE_FILE = 'state.json';
export const EVOLUTION_FILE = 'evolution.jsonl';

// ---- 协议常数(docs/13 §4,可调) ----
export const EVAL_RUNS_DEFAULT = 3; // 候选试跑局数(取中位数)
export const EPSILON = 0.05; // 提交最小提升阈值
export const COOLDOWN_ITERS = 5; // 字段回滚后的冷却迭代数
export const PLATEAU_WINDOW = 3; // 连续无进展局数触发求助
export const MAX_PATCH_FIELDS = 3; // 单次补丁最多字段数
export const MAX_NUMERIC_STEP = 0.2; // 数值字段单次步进 ≤20%
export const MAX_WINDOW_STEP_PX = 8; // 触发窗端点单次移动 ≤8px
const ANALYZED_CAP = 50;
const HISTORY_CAP = 50;

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

export type EvoState = {
  /** 已完成训练局数(Qwen 提案周期的计数) */
  iteration: number;
  /** champion 最近一次评估中位分;null = 尚未测量(首个候选直接转正) */
  championScore: number | null;
  /** 字段 → 冷却截止迭代号(含);回滚过的字段冷却期内禁止再改 */
  cooldowns: Record<string, number>;
  /** 已做过 Qwen 分析的 runId(幂等,滚动 50) */
  analyzedRunIds: string[];
  /** 上次分析过的失败签名(死因@地标);null = 尚无 */
  lastDeathSignature: string | null;
  bestMaxXCol: number;
  plateauStreak: number;
};

export function initialEvoState(): EvoState {
  return {
    iteration: 0,
    championScore: null,
    cooldowns: {},
    analyzedRunIds: [],
    lastDeathSignature: null,
    bestMaxXCol: 0,
    plateauStreak: 0,
  };
}

export function parseEvoState(raw: string | null): { state: EvoState; issues: string[] } {
  if (raw === null || raw.trim() === '') return { state: initialEvoState(), issues: [] };
  try {
    const rec = JSON.parse(raw) as Record<string, unknown>;
    return {
      state: {
        iteration: typeof rec['iteration'] === 'number' ? rec['iteration'] : 0,
        championScore: typeof rec['championScore'] === 'number' ? rec['championScore'] : null,
        cooldowns:
          typeof rec['cooldowns'] === 'object' && rec['cooldowns'] !== null
            ? (rec['cooldowns'] as Record<string, number>)
            : {},
        analyzedRunIds: Array.isArray(rec['analyzedRunIds'])
          ? (rec['analyzedRunIds'] as string[]).slice(-ANALYZED_CAP)
          : [],
        lastDeathSignature:
          typeof rec['lastDeathSignature'] === 'string' ? rec['lastDeathSignature'] : null,
        bestMaxXCol: typeof rec['bestMaxXCol'] === 'number' ? rec['bestMaxXCol'] : 0,
        plateauStreak: typeof rec['plateauStreak'] === 'number' ? rec['plateauStreak'] : 0,
      },
      issues: [],
    };
  } catch {
    return { state: initialEvoState(), issues: ['state.json 损坏,簿记已重置'] };
  }
}

// ================= Qwen 触发器(docs/13 §3.1:有新信息才调) =================

export type QwenTrigger = { invoke: boolean; reason: string };

/** 失败签名 = 最常见死因 @ 其首次发生的地标;无死亡 → null。 */
export function deathSignature(report: PostmortemReport): string | null {
  const top = report.deathCauses[0];
  if (top === undefined) return null;
  const first = report.deaths.find((d) => d.cause === top.cause);
  return `${top.cause}@${first?.landmark ?? '?'}`;
}

export function shouldInvokeQwen(
  state: EvoState,
  report: PostmortemReport,
  runId: string,
): QwenTrigger {
  if (state.analyzedRunIds.includes(runId)) {
    return { invoke: false, reason: 'already-analyzed(同一 runId 幂等)' };
  }
  if (report.outcome === 'win') return { invoke: true, reason: 'win(沉淀成功经验)' };
  const sig = deathSignature(report);
  if (sig !== null && sig !== state.lastDeathSignature) {
    return { invoke: true, reason: `new-death(${sig})` };
  }
  if (state.plateauStreak >= PLATEAU_WINDOW) {
    return { invoke: true, reason: `plateau(连续 ${state.plateauStreak} 局无进展)` };
  }
  return {
    invoke: false,
    reason: sig === null ? 'no-signal(无死亡无通关)' : 'repeat-failure(同签名去重)',
  };
}

/** 每局结束后更新簿记(无论是否调了 Qwen)。返回新状态,不原地改。 */
export function recordRun(
  state: EvoState,
  report: PostmortemReport,
  runId: string,
  analyzed: boolean,
): EvoState {
  const progressed = report.maxXCol > state.bestMaxXCol;
  const sig = deathSignature(report);
  return {
    ...state,
    iteration: state.iteration + 1,
    bestMaxXCol: progressed ? report.maxXCol : state.bestMaxXCol,
    plateauStreak: progressed ? 0 : state.plateauStreak + 1,
    lastDeathSignature: analyzed && sig !== null ? sig : state.lastDeathSignature,
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
 * 变更幅度限制(docs/13 §4.2-2):小步爬山可回溯,大步跳进易出甜区。
 * - 最多保留 MAX_PATCH_FIELDS 个字段(按相对变化幅度从大到小);
 * - 数值标量步进钳到当前的 ±20%(小于 1 的抖动视为无变化直接丢);
 * - 触发窗端点各钳 ±8px;布尔直接放行。
 */
export function limitPatch(
  patch: PolicyProfile,
  current: PolicyProfile,
): { policy: PolicyProfile; issues: string[] } {
  const issues: string[] = [];
  const out: PolicyProfile = { ...current };
  const changed: { key: keyof PolicyProfile; magnitude: number }[] = [];
  for (const key of Object.keys(DEFAULT_POLICY) as (keyof PolicyProfile)[]) {
    const a = current[key];
    const b = patch[key];
    if (typeof a === 'boolean' && typeof b === 'boolean') {
      if (a !== b) changed.push({ key, magnitude: 1 });
      continue;
    }
    if (typeof a === 'number' && typeof b === 'number') {
      const rel = Math.abs(b - a) / Math.max(1, Math.abs(a));
      if (rel > 0.001) changed.push({ key, magnitude: rel });
      continue;
    }
    if (Array.isArray(a) && Array.isArray(b) && (a[0] !== b[0] || a[1] !== b[1])) {
      changed.push({ key, magnitude: Math.max(Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1])) / 16 });
    }
  }
  changed.sort((x, y) => y.magnitude - x.magnitude);
  for (const [i, c] of changed.entries()) {
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
    } else if (Array.isArray(a) && Array.isArray(b)) {
      const clamp = (from: number, to: number): number =>
        from + Math.max(-MAX_WINDOW_STEP_PX, Math.min(MAX_WINDOW_STEP_PX, to - from));
      const w: [number, number] = [clamp(a[0], b[0]), clamp(a[1], b[1])];
      if (w[0] !== b[0] || w[1] !== b[1]) {
        issues.push(`限幅:${c.key} 端点移动超 ±${MAX_WINDOW_STEP_PX}px,钳到 [${w[0]},${w[1]}]`);
      }
      (out as Record<string, unknown>)[c.key] = w;
    } else {
      (out as Record<string, unknown>)[c.key] = b;
    }
  }
  return { policy: out, issues };
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
  v: 1;
  ts: string;
  iter: number;
  runId: string | null;
  actor: 'qwen' | 'user' | 'system';
  action: 'evaluate' | 'propose' | 'trial' | 'commit' | 'rollback' | 'skip';
  reason: string;
  patch?: Record<string, { from: unknown; to: unknown }>;
  score?: { champion: number | null; candidate: number | null };
  issues?: string[];
};

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
      if (e.v === 1 && typeof e.action === 'string') out.push(e);
    } catch {
      // 坏行跳过:审计日志宁可缺行不可崩
    }
  }
  return out;
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
