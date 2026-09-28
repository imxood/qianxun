/**
 * qwen.ts 纯逻辑测试(docs/14 §5):提示词构成(触发原因/过程 digest)、
 * 三输出解析(playbook / policy_patch / insight)与钳制。
 */

import { describe, expect, it } from 'vitest';

import { DEFAULT_POLICY } from './policy';
import { INSIGHT_MAX_CHARS, PLAYBOOK_MAX_CHARS, buildRefineMessages, parseRefine } from './qwen';
import type { PostmortemReport } from './postmortem';

function report(over: Partial<PostmortemReport> = {}): PostmortemReport {
  return {
    id: 's1',
    outcome: 'incomplete',
    attempts: 2,
    score: 0,
    maxXCol: 60,
    progressPct: 28,
    durationTicks: 1800,
    decisions: 20,
    gates: { exec: 14, reSense: 2, escalate: 3, stale: 1, guard: 0 },
    execRate: 0.7,
    escalateRate: 0.15,
    staleRate: 0.05,
    avgConf: 0.42,
    p50LatencyMs: 230,
    deaths: [
      {
        cause: 'goomba',
        col: 28,
        landmark: 'pipe@28(h2)',
        tick: 700,
        context: [{ action: 'run_right', conf: 0.3, gate: 'EXECUTE', col: 27 }],
      },
    ],
    deathCauses: [{ cause: 'goomba', count: 1 }],
    stallSites: [],
    thrashSites: [{ col: 40, count: 4 }],
    churnSites: [],
    firstDeathTick: 700,
    actionMix: [],
    vetoes: 0,
    timeline: [],
    hypotheses: [],
    suggestions: [],
    ...over,
  };
}

describe('qwen 提示词', () => {
  it('含触发原因与过程 digest;重复死亡加标注(docs/14 §2)', () => {
    const msgs = buildRefineMessages({
      playbook: '旧手册',
      report: report(),
      policy: DEFAULT_POLICY,
      history: [],
      reason: 'death-repeat(goomba@pipe@28(h2))',
    });
    expect(msgs).toHaveLength(2);
    const user = msgs[1]?.content ?? '';
    expect(user).toContain('【触发原因】death-repeat');
    expect(user).toContain('重复死亡');
    expect(user).toContain('【本局过程 digest】');
    expect(user).toContain('门控抖动');
    const sys = msgs[0]?.content ?? '';
    expect(sys).toContain('obsProfileCols');
    expect(sys).toContain('insight');
  });

  it('无 reason 也能构造(向后兼容)', () => {
    const msgs = buildRefineMessages({
      playbook: '',
      report: report(),
      policy: DEFAULT_POLICY,
      history: [],
    });
    expect(msgs[1]?.content).toContain('(空——这是第一局)');
  });
});

describe('qwen 输出解析', () => {
  const fb = { playbook: '旧手册', policy: DEFAULT_POLICY };

  it('三输出齐全:playbook 截断 / patch sanitize / insight ≤200 字', () => {
    const content = `前导废话 {"playbook":"新手册","policy_patch":{"gateExecute":0.9,"obsProfileCols":24},"insight":"${'经'.repeat(300)}"} 尾巴`;
    const r = parseRefine(content, fb);
    expect(r.playbook).toBe('新手册');
    // gateExecute 0.9 越界 → 钳回 ≤0.5
    expect(r.policy.gateExecute).toBeLessThanOrEqual(0.5);
    expect(r.policy.obsProfileCols).toBe(24);
    expect(r.insight.length).toBe(INSIGHT_MAX_CHARS);
  });

  it('非 JSON 输出:保留原手册与策略,insight 为空,记 issue', () => {
    const r = parseRefine('完全不是 JSON', fb);
    expect(r.playbook).toBe('旧手册');
    expect(r.policy).toEqual(DEFAULT_POLICY);
    expect(r.insight).toBe('');
    expect(r.issues).toEqual(['输出不含 JSON,保留原状']);
  });

  it('缺 insight 字段 → 空字符串;playbook 超长按 1600 截断', () => {
    const r = parseRefine(JSON.stringify({ playbook: 'x'.repeat(2000), policy_patch: {} }), fb);
    expect(r.playbook.length).toBe(PLAYBOOK_MAX_CHARS);
    expect(r.insight).toBe('');
  });
});
