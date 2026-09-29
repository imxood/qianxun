/**
 * 本地 Qwen 客户端(docs/12 双脑"慢思"侧,仅 :17230 本地服务)。
 *
 * v3(docs/15 §6):输入 = 触发原因 + 全局 digest + **死亡级全量轨迹与 Laya
 * 收发明细**;输出 = ①playbook ②沙箱内 policy_patch(数值+消息设计,单维度)
 * ③death_diagnosis(进载荷闭环,禁回控制路径)④可证伪 insight(必填)。
 * 边界靠解析器硬隔离:顶层键白名单、静态性声明、值域钳制——不靠模型自觉。
 * 纯函数 + 注入 fetch(UI webview 与 node 无头闭环共用);失败向上抛由调度器降级。
 */

import type { PostmortemReport, PmDecisionRow, SessionInput } from './postmortem';
import { attemptRows, isAnomalyRow, processDigest, traceCsv, trimTrace } from './postmortem';
import { sanitizePolicy, type PolicyProfile } from './policy';

export const PLAYBOOK_MAX_CHARS = 1600;
export const INSIGHT_MAX_CHARS = 200;
export const QWEN_ENDPOINT = 'http://127.0.0.1:17230';
/** 单次分析超时(docs/15 R2):本地服务挂起不得占死队列。 */
export const QWEN_TIMEOUT_MS = 60_000;
/** 载荷字符预算(≈6k token,按 chars/4 折算,不用分词器 → 同输入必同输出)。 */
export const PAYLOAD_CHAR_BUDGET = 24_000;

export type HistoryRow = { iteration: number; won: boolean; maxXCol: number; ticks: number };

/** 可证伪 insight(docs/15 §6.5):缺 metric/证据的 claim 视同空,走重试。 */
export type LayaInsight = {
  kind: 'claim' | 'maintain' | 'refuted';
  claim: string;
  field?: string;
  metric?: string;
  direction?: 'up' | 'down';
  evidenceIter?: number[];
};

export const INSIGHT_METRICS = [
  'winRate',
  'repeatRate',
  'execRate',
  'staleRate',
  'avgConf',
  'p50ms',
] as const;

export type DeathDiagnosis = {
  deathKey: string;
  rootCause: string;
  responsibleTick: number | null;
  fix: string;
};

function extractJson(text: string): Record<string, unknown> | null {
  const a = text.indexOf('{');
  const b = text.lastIndexOf('}');
  if (a < 0 || b <= a) return null;
  try {
    return JSON.parse(text.slice(a, b + 1)) as Record<string, unknown>;
  } catch {
    return null;
  }
}

// ================= 提示词 =================

const SELECTION_GUIDE = [
  '消息设计字段选型指引(值只能取注册变体,不得自创文本):',
  '- obsThreatFormat named|rows|off:空转或"看不见威胁"型死亡多 → rows;威胁识别正常 → 别动。',
  '- obsProgressStyle full|pct|off:pct 去掉绝对像素一般够用;通关在望 → 不许动。',
  '- obsHintStyle full|terse|off:token 紧张且动作选择清晰 → terse;跳管/宽坑失败率回升 → 回 full。',
  '- obsInstructionVariant default|concise|checklist:concise 省 token;checklist 内置规则,仅开局即死模式考虑;不确定 → default。',
  '- obsStateExtra ⊆[lastAction,heldTicks,stallTicks] ≤2:门控抖动/空转多 → stallTicks;动作横跳 → lastAction+heldTicks。',
  '纪律:单次 patch 至多 1 个消息设计字段(与数值字段不同车);设计字段改后至少观测完整评估窗才许下结论。',
].join('\n');

export function buildRefineMessages(input: {
  playbook: string;
  report: PostmortemReport;
  policy: PolicyProfile;
  history: HistoryRow[];
  /** 触发原因(docs/15 §6.1):death / death-repeat / win / plateau。 */
  reason?: string;
  /** 死亡级全量载荷(§6.3);win/plateau 可省。 */
  deathPayload?: string;
  /** 上次同签名诊断+补丁(death-repeat 必带,回答"上轮为何没生效")。 */
  lastDiagnosis?: DeathDiagnosis[];
  prevPatch?: Record<string, { from: unknown; to: unknown }>;
  /** 已沉淀的 insight 摘要(去重参考)。 */
  lastInsights?: string[];
  /** 本轮合法 deathKey 清单(death_diagnosis 必须逐字取自这里)。 */
  validDeathKeys?: readonly string[];
}): Array<{ role: string; content: string }> {
  const system = [
    '你是自主平台跳跃游戏的局间策略优化代理(本地,离线)。你是局间策略优化者,不是逐拍玩家——边界由解析器保证。',
    '每局结束输出一个 JSON 对象,四个键:',
    '1. playbook:给下一局的策略手册,≤1000 字,只写可执行经验;必须指出至少一个过程异常(直接掉崖/原地空转/门控抖动/反复同点死亡)及处置;',
    '2. policy_patch:只允许声明表内的字段——',
    '   数值/布尔:intervalMs 150-600 | gateExecute 0.05-0.5 | gateEscalate 0.02-0.3 |',
    '   vetoDistPx 6-40 | backoffTicks 10-60 | vetoEnabled | gapWindow/pipeWindow/goombaWindow/stairWindow [负,正]px |',
    '   obsProfileCols 8-24 | obsThreatsLookPx 96-288 | obsIncludePose | obsIncludeSubgoal;',
    '   消息设计(枚举,只可选值):obsThreatFormat named|rows|off | obsProgressStyle full|pct|off |',
    '   obsHintStyle full|terse|off | obsInstructionVariant default|concise|checklist |',
    '   obsStateExtra ⊆[lastAction,heldTicks,stallTicks] ≤2 项。',
    '   单维度:数值与消息设计不得同车;≤3 字段;越界会被钳制。',
    '3. death_diagnosis:数组,每项 {"deathKey","rootCause","responsibleTick","fix"}(只进复盘,不产生逐拍指令);',
    '4. insight:必填可证伪对象 {"kind":"claim","claim":"≤150字","field":"涉字段","metric":"winRate|repeatRate|execRate|staleRate|avgConf|p50ms","direction":"up|down","evidenceIter":[迭代号]}。',
    '   确无新发现时 {"kind":"maintain","claim":"维持:<哪个指标支持维持>"}。缺 metric 或 evidenceIter 的 claim 视同无效。',
    '事实纪律:【当前策略与消息设计】是唯一事实;手册/历史经验中与之矛盾的描述一律以它为准——被否决/未生效的旧提案不得当作已生效前提继续推理。',
    SELECTION_GUIDE,
    '输出只能是这一个 JSON 对象,不要解释。目标:通关率提高、通关耗时缩短。',
  ].join('\n');
  const repeat =
    input.reason?.startsWith('death-repeat') === true
      ? '\n注意:这是同一死因签名的重复死亡——上一轮优化没有生效,请聚焦"为什么没生效",不要重复描述死法。'
      : '';
  const user: string[] = [`【触发原因】${input.reason ?? '(未标注)'}${repeat}`];
  if (input.validDeathKeys && input.validDeathKeys.length > 0) {
    user.push(
      `【本轮 deathKey 清单(death_diagnosis 必须逐字取自这里)】${input.validDeathKeys.join(', ')}`,
    );
  }
  if (input.deathPayload !== undefined) {
    user.push(`【死亡级全量载荷(归因素材,不得产出逐拍指令)】\n${input.deathPayload}`);
  } else {
    user.push(`【本局过程 digest】\n${JSON.stringify(processDigest(input.report))}`);
  }
  if (input.lastDiagnosis && input.lastDiagnosis.length > 0) {
    user.push(`【上次同签名诊断(请回答为何未生效)】\n${JSON.stringify(input.lastDiagnosis)}`);
  }
  if (input.prevPatch && Object.keys(input.prevPatch).length > 0) {
    user.push(`【上次同签名补丁】\n${JSON.stringify(input.prevPatch)}`);
  }
  if (input.lastInsights && input.lastInsights.length > 0) {
    user.push(`【最近经验(勿重复)】\n${input.lastInsights.map((s) => `- ${s}`).join('\n')}`);
  }
  user.push(`【策略手册(当前)】\n${input.playbook || '(空——这是第一局)'}`);
  user.push(`【最近迭代】\n${JSON.stringify(input.history.slice(-5))}`);
  user.push(`【当前策略与消息设计】\n${JSON.stringify(input.policy)}`);
  return [
    { role: 'system', content: system },
    { role: 'user', content: user.join('\n\n') },
  ];
}

// ================= 死亡级载荷组装(docs/15 §6.3) =================

export type SentRowLike = {
  row: PmDecisionRow;
  state: string;
  questions?: string;
  tick: number;
  attempt: number;
};

export type DeathPayloadInput = {
  report: PostmortemReport;
  session: SessionInput;
  sent: readonly SentRowLike[];
  attempt: number;
  cause: string;
  landmark: string;
};

function anomalyCsvRow(r: PmDecisionRow): string {
  return traceCsv([r])[0] ?? '';
}

/** 载荷组装:①digest ②轨迹 ③死前 12 拍明细 ④异常行 ⑤其它命摘要 ⑥上下文由调用方拼。 */
export function buildDeathPayload(input: DeathPayloadInput): {
  text: string;
  sections: Record<string, string>;
  overBudget: boolean;
} {
  const { report, session, sent, attempt } = input;
  const sections: Record<string, string> = {};
  // ③ 死前 12 拍 Laya 收发明细(PROTECTED:只降密度到 8 拍,不归零)
  const deathSent = sent.filter((s) => s.attempt === attempt).slice(-12);
  const gate = (g: string | undefined): string =>
    g === 'EXECUTE' ? 'EX' : g === 'RE_SENSE' ? 'RS' : g === 'ESCALATE' ? 'ES' : (g ?? '?');
  const detail3 = deathSent
    .map((s) => {
      const r = s.row;
      const decision =
        r.applied === false
          ? `=> ${r.action ?? '?'} .${(r.conf ?? 0).toFixed(2)} ${gate(r.gate)} ${r.latencyMs ?? 0}ms stale!`
          : `=> ${r.action ?? '?'} .${(r.conf ?? 0).toFixed(2)} ${gate(r.gate)} ${r.latencyMs ?? 0}ms`;
      return `t${s.tick} ${s.state}\n${decision}`;
    })
    .join('\n');
  const detailMin = deathSent.slice(-8);
  const detail3Min = detailMin
    .map((s) => `t${s.tick} ${s.state}\n=> ${s.row.action ?? '?'} .${(s.row.conf ?? 0).toFixed(2)}`)
    .join('\n');
  // ② 本命全量轨迹(确定性裁剪)
  const rows = attemptRows(session, attempt);
  const full2 = traceCsv(trimTrace(rows, 120)).join('\n');
  const half2 = traceCsv(trimTrace(rows, 60)).join('\n');
  // ④ 全程异常行(PROTECTED:cap 6 全量 + 8 条 CSV 降级)
  const anomalies = session.decisions.filter(isAnomalyRow);
  const anomalyFull = anomalies
    .slice(0, 6)
    .map((r) => `${anomalyCsvRow(r)}\n  ${sent.find((x) => x.tick === r.tick)?.state ?? ''}`)
    .join('\n');
  const anomalyCsv = anomalies.slice(6, 14).map(anomalyCsvRow).join('\n');
  // ⑤ 其它命摘要(TRIMMABLE 最先)
  const otherAttempts = new Set(
    session.decisions
      .map((d) => d.attempt)
      .filter((a): a is number => a !== undefined && a !== attempt),
  );
  const other5 = [...otherAttempts]
    .map((a) => {
      const rs = attemptRows(session, a);
      const step = Math.max(1, Math.ceil(rs.length / 4));
      return rs
        .filter((_, i) => i % step === 0)
        .map((r) => `a${a} ${anomalyCsvRow(r)}`)
        .join('\n');
    })
    .join('\n');
  // ① 全局 digest(PROTECTED,最后才压)
  const s1 = `【① 全局 digest】\n${JSON.stringify(processDigest(report))}`;
  const s2 = `【② 本命 #${attempt} 全量轨迹 CSV(tick,col,action,conf,gate,applied,note)】\n${full2}`;
  const s2b = `【② 本命 #${attempt} 轨迹采样(sampled 60)】\n${half2}`;
  const s3 = `【③ 死前 12 拍 Laya 收发明细(死因 ${input.cause} @ ${input.landmark})】\n${detail3}`;
  const s3b = `【③ 死前 8 拍收发摘要(sampled)】\n${detail3Min}`;
  const s4 = `【④ 全程异常拍】\n${[anomalyFull, anomalyCsv].filter(Boolean).join('\n')}`;
  const s5 = other5 ? `【⑤ 其它命摘要】\n${other5}` : '';
  // 组装:PROTECTED ③④ 优先保,TRIMMABLE ⑤→②→①
  let parts = [s3, s4, s5, s2, s1].filter((p) => p !== '');
  if (parts.join('\n\n').length > PAYLOAD_CHAR_BUDGET) parts = [s3, s4, s2b, s1];
  if (parts.join('\n\n').length > PAYLOAD_CHAR_BUDGET) parts = [s3b, s4, s2b, s1];
  const text = parts.join('\n\n');
  sections['①'] = s1;
  sections['②'] = s2;
  sections['③'] = s3;
  sections['④'] = s4;
  sections['⑤'] = s5;
  return { text, sections, overBudget: text.length > PAYLOAD_CHAR_BUDGET };
}

// ================= 输出解析 =================

export function validateInsight(raw: unknown): { insight: LayaInsight | null; reason: string } {
  if (typeof raw === 'string') {
    const claim = raw.trim().slice(0, INSIGHT_MAX_CHARS);
    if (claim === '') return { insight: null, reason: 'insight 为空(必填)' };
    return { insight: null, reason: 'insight 缺可证伪字段(metric/direction)' };
  }
  if (typeof raw !== 'object' || raw === null) {
    return { insight: null, reason: 'insight 缺失(必填)' };
  }
  const o = raw as Record<string, unknown>;
  const kind = o['kind'] === 'maintain' || o['kind'] === 'refuted' ? o['kind'] : 'claim';
  const claim = typeof o['claim'] === 'string' ? o['claim'].trim().slice(0, INSIGHT_MAX_CHARS) : '';
  if (claim === '') return { insight: null, reason: 'insight.claim 为空(必填)' };
  if (kind === 'maintain' || kind === 'refuted') {
    return { insight: { kind, claim }, reason: '' };
  }
  const metric = typeof o['metric'] === 'string' ? o['metric'] : '';
  if (!(INSIGHT_METRICS as readonly string[]).includes(metric)) {
    return { insight: null, reason: `insight.metric 非法:${metric || '(缺)'}` };
  }
  const direction = o['direction'] === 'down' ? 'down' : o['direction'] === 'up' ? 'up' : '';
  if (direction === '') return { insight: null, reason: 'insight.direction 缺失' };
  const field = typeof o['field'] === 'string' ? o['field'] : undefined;
  const evidenceIter = Array.isArray(o['evidenceIter'])
    ? (o['evidenceIter'] as unknown[]).filter((x): x is number => typeof x === 'number')
    : undefined;
  // claim 必须可查证:没有 evidenceIter 就无法对着 runs/ 归档复核(docs/15 §9)
  if (!evidenceIter || evidenceIter.length === 0) {
    return { insight: null, reason: 'insight.evidenceIter 缺失(claim 须指向真实迭代号)' };
  }
  return {
    insight: {
      kind: 'claim',
      claim,
      field,
      metric,
      direction: direction as 'up' | 'down',
      evidenceIter,
    },
    reason: '',
  };
}

export type ParseRefineResult = {
  playbook: string;
  policy: PolicyProfile;
  insight: LayaInsight | null;
  insightIssue: string;
  diagnosis: DeathDiagnosis[];
  issues: string[];
};

export function parseRefine(
  content: string,
  fallback: { playbook: string; policy: PolicyProfile },
  opts: { validDeathKeys?: readonly string[] } = {},
): ParseRefineResult {
  const parsed = extractJson(content);
  if (!parsed) {
    return {
      playbook: fallback.playbook,
      policy: fallback.policy,
      insight: null,
      insightIssue: '输出不含 JSON,保留原状',
      diagnosis: [],
      issues: ['输出不含 JSON,保留原状'],
    };
  }
  const issues: string[] = [];
  // 顶层键白名单:越权可观测,不静默(docs/15 §6.6 硬化①)
  for (const k of Object.keys(parsed)) {
    if (!['playbook', 'policy_patch', 'death_diagnosis', 'insight'].includes(k)) {
      issues.push(`越权顶层键 ${k} 已丢弃`);
    }
  }
  const playbook = String(parsed['playbook'] ?? fallback.playbook).slice(0, PLAYBOOK_MAX_CHARS);
  const patch =
    typeof parsed['policy_patch'] === 'object' && parsed['policy_patch'] !== null
      ? (parsed['policy_patch'] as Record<string, unknown>)
      : {};
  const insightCheck = validateInsight(parsed['insight']);
  if (insightCheck.reason !== '') issues.push(insightCheck.reason);
  // death_diagnosis:deathKey 幻觉校验(docs/15 R14)
  const diagnosis: DeathDiagnosis[] = [];
  if (Array.isArray(parsed['death_diagnosis'])) {
    for (const d of parsed['death_diagnosis'] as unknown[]) {
      const o = d as Record<string, unknown>;
      if (typeof o?.['deathKey'] !== 'string') continue;
      if (opts.validDeathKeys && !opts.validDeathKeys.includes(o['deathKey'])) {
        issues.push(`death_diagnosis.deathKey 幻觉:${o['deathKey']}`);
        continue;
      }
      diagnosis.push({
        deathKey: o['deathKey'],
        rootCause: String(o['rootCause'] ?? ''),
        responsibleTick: typeof o['responsibleTick'] === 'number' ? o['responsibleTick'] : null,
        fix: String(o['fix'] ?? ''),
      });
    }
  }
  const { policy: policy2, issues: policyIssues } = sanitizePolicy({
    ...fallback.policy,
    ...patch,
  });
  return {
    playbook,
    policy: policy2,
    insight: insightCheck.insight,
    insightIssue: insightCheck.reason,
    diagnosis,
    issues: [...issues, ...policyIssues],
  };
}

/** 一次「局后分析」:调用本地 Qwen 并钳制其输出。失败向上抛,由调度器降级。 */
export async function refineViaQwen(input: {
  report: PostmortemReport;
  playbook: string;
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
  const res = await (input.fetchImpl ?? fetch)(
    `${input.endpoint ?? QWEN_ENDPOINT}/v1/chat/completions`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        messages: buildRefineMessages(input),
        temperature: 0.2,
        max_tokens: 1400,
        chat_template_kwargs: { enable_thinking: false },
      }),
      signal: input.signal ?? AbortSignal.timeout(QWEN_TIMEOUT_MS),
    },
  );
  if (!res.ok) throw new Error(`qwen http ${res.status}`);
  const j = (await res.json()) as { choices?: Array<{ message?: { content?: string } }> };
  const content = j.choices?.[0]?.message?.content ?? '';
  return parseRefine(
    content,
    { playbook: input.playbook, policy: input.policy },
    {
      validDeathKeys: input.validDeathKeys,
    },
  );
}
