/**
 * 本地 Qwen 客户端(docs/12 双脑的"慢思"侧,仅 :17230 本地服务)。
 *
 * 职责单一:把「限长策略手册 + 本局复盘 + 当前策略」交给本地 Qwen,收回
 * ①新版手册(≤1600 字,滚动上下文)②沙箱内的策略补丁。
 * 纯函数 + 注入 fetch(UI webview 与 node 无头闭环共用同一套提示词/解析);
 * 输出一律经 sanitizePolicy 钳制——agent 自由迭代,但改不坏游戏。
 */

import type { PostmortemReport } from './postmortem';
import { sanitizePolicy, type PolicyProfile } from './policy';

export const PLAYBOOK_MAX_CHARS = 1600;
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

function compactReport(r: PostmortemReport): Record<string, unknown> {
  return {
    结果: r.outcome,
    尝试: r.attempts,
    最远列: r.maxXCol,
    死因: r.deathCauses,
    停滞: r.stallSites,
    EXEC率: r.execRate,
    ESCALATE率: r.escalateRate,
    stale率: r.staleRate,
    veto: r.vetoes,
    avgConf: r.avgConf,
    p50ms: r.p50LatencyMs,
    时间线: r.timeline.slice(-6),
  };
}

export function buildRefineMessages(input: {
  playbook: string;
  report: PostmortemReport;
  policy: PolicyProfile;
  history: HistoryRow[];
}): Array<{ role: string; content: string }> {
  const system = [
    '你是自主平台跳跃游戏的策略优化代理(本地,离线)。每局结束后你更新两样东西:',
    '1. playbook:给下一局的自己看的策略手册,≤1000 字,只写可执行的经验(哪里该提前起跳、哪个门控怎么调、失败模式的规避),不要空话;',
    '2. policy_patch:只允许这些键与值域——',
    '   intervalMs 150-600 | gateExecute 0.05-0.5 | gateEscalate 0.02-0.3 |',
    '   vetoDistPx 6-40 | backoffTicks 10-60 | vetoEnabled bool |',
    '   gapWindow/pipeWindow/goombaWindow/stairWindow 各 [负数,正数] px 触发窗。',
    '越界会被钳制。只输出一个 JSON 对象,格式 {"playbook":"...","policy_patch":{...}},不要任何解释。',
    '目标:通关成功率提高、通关耗时缩短。常见手段:近身小怪死亡多 → 调 vetoDistPx 与 goombaWindow;',
    '管道前反复失败 → pipeWindow 提前量与 intervalMs(拍太密浪费延迟,太疏跳跃弧断裂)。',
  ].join('\n');
  const user = [
    `【策略手册(当前)】\n${input.playbook || '(空——这是第一局)'}`,
    `【最近迭代】\n${JSON.stringify(input.history.slice(-5))}`,
    `【本局复盘】\n${JSON.stringify(compactReport(input.report))}`,
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
): { playbook: string; policy: PolicyProfile; issues: string[] } {
  const parsed = extractJson(content);
  if (!parsed)
    return {
      playbook: fallback.playbook,
      policy: fallback.policy,
      issues: ['输出不含 JSON,保留原状'],
    };
  const playbook = String(parsed.playbook ?? fallback.playbook).slice(0, PLAYBOOK_MAX_CHARS);
  const patch =
    typeof parsed.policy_patch === 'object' && parsed.policy_patch !== null
      ? (parsed.policy_patch as Record<string, unknown>)
      : {};
  const { policy, issues } = sanitizePolicy({ ...fallback.policy, ...patch });
  return { playbook, policy, issues };
}

/** 一次「局后分析」:调用本地 Qwen 并钳制其输出。任何失败向上抛,由调用方降级。 */
export async function refineViaQwen(input: {
  report: PostmortemReport;
  playbook: string;
  policy: PolicyProfile;
  history: HistoryRow[];
  endpoint?: string;
  fetchImpl?: typeof fetch;
}): Promise<{ playbook: string; policy: PolicyProfile; issues: string[] }> {
  const res = await (input.fetchImpl ?? fetch)(
    `${input.endpoint ?? QWEN_ENDPOINT}/v1/chat/completions`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        messages: buildRefineMessages(input),
        temperature: 0.2,
        max_tokens: 800,
        chat_template_kwargs: { enable_thinking: false },
      }),
    },
  );
  if (!res.ok) throw new Error(`qwen http ${res.status}`);
  const j = (await res.json()) as { choices?: Array<{ message?: { content?: string } }> };
  const content = j.choices?.[0]?.message?.content ?? '';
  return parseRefine(content, { playbook: input.playbook, policy: input.policy });
}
