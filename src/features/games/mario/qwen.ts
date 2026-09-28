/**
 * 本地 Qwen 客户端(docs/12 双脑的"慢思"侧,仅 :17230 本地服务)。
 *
 * 职责单一:把「限长策略手册 + 本局全量过程 digest + 当前策略」交给本地
 * Qwen,收回 ①新版手册(≤1600 字,滚动上下文)②沙箱内的策略补丁(含观测
 * 模式字段)③一条可泛化的 Laya 使用经验(insight,docs/14 §5)。
 * 纯函数 + 注入 fetch(UI webview 与 node 无头闭环共用同一套提示词/解析);
 * 输出一律经 sanitizePolicy 钳制——agent 自由迭代,但改不坏游戏。
 */

import type { PostmortemReport } from './postmortem';
import { processDigest } from './postmortem';
import { sanitizePolicy, type PolicyProfile } from './policy';

export const PLAYBOOK_MAX_CHARS = 1600;
export const INSIGHT_MAX_CHARS = 200;
export const QWEN_ENDPOINT = 'http://127.0.0.1:17230';

export type HistoryRow = { iteration: number; won: boolean; maxXCol: number; ticks: number };

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

export function buildRefineMessages(input: {
  playbook: string;
  report: PostmortemReport;
  policy: PolicyProfile;
  history: HistoryRow[];
  /** 触发原因(docs/14 §2):death / death-repeat / win / plateau。 */
  reason?: string;
  /** 已沉淀的 insight 数(提示 Qwen 别重复写)。 */
  insightCount?: number;
}): Array<{ role: string; content: string }> {
  const system = [
    '你是自主平台跳跃游戏的策略优化代理(本地,离线)。每局结束后你更新三样东西:',
    '1. playbook:给下一局的自己看的策略手册,≤1000 字,只写可执行的经验(哪里该提前起跳、哪个门控怎么调、失败模式的规避),不要空话;',
    '2. policy_patch:只允许这些键与值域——',
    '   intervalMs 150-600 | gateExecute 0.05-0.5 | gateEscalate 0.02-0.3 |',
    '   vetoDistPx 6-40 | backoffTicks 10-60 | vetoEnabled bool |',
    '   gapWindow/pipeWindow/goombaWindow/stairWindow 各 [负数,正数] px 触发窗;',
    '   观测模式(喂给 Laya 模型的数据,docs/14 §4):',
    '   obsProfileCols 8-24(前瞻列数)| obsThreatsLookPx 96-288(威胁前扫距离)|',
    '   obsIncludePose/obsIncludeSubgoal bool(关闭 = 从请求里移除该数据)。',
    '3. insight:一条可泛化的 Laya 模型使用经验,≤150 字,聚焦"喂什么数据/什么参数更合理"',
    '   (如:观测件与置信度的关系、前瞻列数的边际收益、延迟与门控的配合),',
    '   不写游戏战术;无新发现则给空字符串。',
    '越界会被钳制。只输出一个 JSON 对象,格式 {"playbook":"...","policy_patch":{...},"insight":"..."},不要任何解释。',
    '目标:通关成功率提高、通关耗时缩短。必须基于【过程 digest】指出至少一个过程异常',
    '(直接掉崖/原地空转/门控抖动/反复同点死亡)及其处置,写进 playbook。',
    '观测调参指引:空转或"看不见威胁"型死亡多 → 调大 obsThreatsLookPx 或确认观测件没关;',
    '置信度无分辨力 → 考虑关闭冗余观测减 token,或扩大 obsProfileCols 给更多上下文。',
  ].join('\n');
  const repeat =
    input.reason?.startsWith('death-repeat') === true
      ? `\n注意:这是同一死因签名的重复死亡——上一轮优化没有生效,请聚焦"为什么没生效",不要重复描述死法。`
      : '';
  const user = [
    `【触发原因】${input.reason ?? '(未标注)'}${repeat}`,
    `【策略手册(当前)】\n${input.playbook || '(空——这是第一局)'}`,
    `【最近迭代】\n${JSON.stringify(input.history.slice(-5))}`,
    `【本局过程 digest】\n${JSON.stringify(processDigest(input.report))}`,
    `【当前策略】\n${JSON.stringify(input.policy)}`,
  ].join('\n\n');
  return [
    { role: 'system', content: system },
    { role: 'user', content: user },
  ];
}

export function parseRefine(
  content: string,
  fallback: { playbook: string; policy: PolicyProfile },
): { playbook: string; policy: PolicyProfile; insight: string; issues: string[] } {
  const parsed = extractJson(content);
  if (!parsed)
    return {
      playbook: fallback.playbook,
      policy: fallback.policy,
      insight: '',
      issues: ['输出不含 JSON,保留原状'],
    };
  const playbook = String(parsed.playbook ?? fallback.playbook).slice(0, PLAYBOOK_MAX_CHARS);
  const patch =
    typeof parsed.policy_patch === 'object' && parsed.policy_patch !== null
      ? (parsed.policy_patch as Record<string, unknown>)
      : {};
  const insight =
    typeof parsed.insight === 'string' ? parsed.insight.trim().slice(0, INSIGHT_MAX_CHARS) : '';
  const { policy, issues } = sanitizePolicy({ ...fallback.policy, ...patch });
  return { playbook, policy, insight, issues };
}

/** 一次「局后分析」:调用本地 Qwen 并钳制其输出。任何失败向上抛,由调用方降级。 */
export async function refineViaQwen(input: {
  report: PostmortemReport;
  playbook: string;
  policy: PolicyProfile;
  history: HistoryRow[];
  reason?: string;
  insightCount?: number;
  endpoint?: string;
  fetchImpl?: typeof fetch;
}): Promise<{ playbook: string; policy: PolicyProfile; insight: string; issues: string[] }> {
  const res = await (input.fetchImpl ?? fetch)(
    `${input.endpoint ?? QWEN_ENDPOINT}/v1/chat/completions`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        messages: buildRefineMessages(input),
        temperature: 0.2,
        max_tokens: 1000,
        chat_template_kwargs: { enable_thinking: false },
      }),
    },
  );
  if (!res.ok) throw new Error(`qwen http ${res.status}`);
  const j = (await res.json()) as { choices?: Array<{ message?: { content?: string } }> };
  const content = j.choices?.[0]?.message?.content ?? '';
  return parseRefine(content, { playbook: input.playbook, policy: input.policy });
}
