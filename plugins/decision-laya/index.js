/**
 * decision-laya — Laya System-1 决策引擎的 DSH Host 插件。
 *
 * 职责红线(docs/09 §1):只产出结构化 Decision + policy_hint,不执行任何动作;
 * 模型不是权限系统,真正执行由 Policy 决定。
 *
 * 依赖:本机 laya-server sidecar(127.0.0.1 only,见 src-tauri/crates/laya-server)。
 * 未启动时工具返回结构化错误,不炸 session。
 */

const ENDPOINT = process.env.LAYA_SERVER_URL ?? 'http://127.0.0.1:10230';
const TIMEOUT_MS = Number(process.env.LAYA_TIMEOUT_MS ?? 30_000);
// policy_hint v1:所有答案置信度的最小值达到阈值 → proceed,否则 escalate。
const PROCEED_MIN_CONFIDENCE = Number(process.env.LAYA_PROCEED_MIN_CONFIDENCE ?? 0.6);

function computePolicyHint(answers) {
  let min = 1;
  for (const a of Object.values(answers ?? {})) {
    const c = typeof a?.confidence === 'number' ? a.confidence : 1;
    if (c < min) min = c;
  }
  return {
    policy_version: 'local-v1-min-confidence',
    min_confidence: Number(min.toFixed(4)),
    hint: min >= PROCEED_MIN_CONFIDENCE ? 'proceed' : 'escalate',
  };
}

const tool = {
  name: 'laya_decide',
  description:
    'Laya System-1 结构化决策(本机模型,毫秒级,不生成文本)。对给定的 state 与 typed questions ' +
    '做一次 forward,返回每个问题的 choice/score/noul 与校准概率。' +
    'questions 形如 {"department":{"type":"choice","instructions":"...","criteria":{"billing":"…","technical":"…"}}, ' +
    '"urgent":{"type":"score","instructions":"...","criteria":["低","中","高"]}, ' +
    '"refund":{"type":"noul","instructions":"..."}}。' +
    '仅用于分类/打分/是非判断等结构化决策;不用于生成或推理。返回含 policy_hint,执行与否由 Policy 决定。',
  parameters: {
    type: 'object',
    properties: {
      state: {
        oneOf: [{ type: 'string' }, { type: 'object' }],
        description: '待判断的状态:一段文本或一个 JSON 对象(如工单/邮件/agent trace 摘要)',
      },
      questions: {
        type: 'object',
        description: '问题表:qid → {type: choice|score|noul, instructions, criteria}',
      },
    },
    required: ['state', 'questions'],
  },
  output: {
    schema: { type: 'object' },
    render(_args, value) {
      const answers = value?.answers ?? {};
      const lines = [`policy_hint: ${value?.policy_hint?.hint} (min_confidence=${value?.policy_hint?.min_confidence})`];
      for (const [qid, a] of Object.entries(answers)) {
        if (a.type === 'choice') {
          lines.push(`${qid}: ${a.choice} (P=${a.probabilities?.[a.choice]}, confidence=${a.confidence})`);
        } else if (a.type === 'score') {
          lines.push(`${qid}: score=${a.score}/${Object.keys(a.probabilities ?? {}).length - 1} (confidence=${a.confidence})`);
        } else if (a.type === 'noul') {
          lines.push(`${qid}: P(true)=${a.noul} (confidence=${a.confidence})`);
        }
      }
      lines.push(`model: ${value?.model} · output_tokens: ${value?.usage?.output_tokens} · latency: ${value?.latency_ms}ms`);
      return [{ type: 'text', text: lines.join('\n') }];
    },
  },
  async execute(args, exec) {
    const signal = exec?.signal ?? undefined;
    let response;
    try {
      response = await fetch(`${ENDPOINT}/predict`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ state: args.state, questions: args.questions }),
        signal: signal ?? AbortSignal.timeout(TIMEOUT_MS),
      });
    } catch (err) {
      const reason = err?.name === 'TimeoutError' || err?.name === 'AbortError'
        ? `laya-server 响应超时(${TIMEOUT_MS}ms)`
        : `laya-server 不可达(${ENDPOINT});请确认 sidecar 已启动: laya-server --model-dir models\\laya-onnx`;
      throw new Error(reason);
    }
    if (!response.ok) {
      const detail = await response.text().catch(() => '');
      throw new Error(`laya-server ${response.status}: ${detail.slice(0, 500)}`);
    }
    const value = await response.json();
    value.policy_hint = computePolicyHint(value.answers);
    return value;
  },
};

export const inject = ['tools'];

export function apply(ctx) {
  const dispose = ctx.tools.register(tool);
  ctx.on?.('dispose', () => dispose?.());
  return () => dispose?.();
}
