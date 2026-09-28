/**
 * evolution.ts 纯逻辑测试(docs/13):信封解析、触发器、评估判定、
 * 冷却期、补丁限幅。这些规则是"越来越优可证明"的纪律层,必须锁死。
 */

import { describe, expect, it } from 'vitest';

import {
  COOLDOWN_ITERS,
  INSIGHTS_MAX_CHARS,
  applyCooldowns,
  appendHistory,
  appendInsight,
  auditLine,
  deathSignature,
  diffPatch,
  evalMedian,
  initialEvoState,
  judgeCandidate,
  limitPatch,
  median,
  obsSnapshotOf,
  parseAuditLines,
  parseEvoState,
  parseHistory,
  parsePolicyFile,
  recordRun,
  runScore,
  shouldInvokeQwen,
  startCooldowns,
  wrapPolicy,
} from './evolution';
import { DEFAULT_POLICY } from './policy';
import type { PostmortemReport } from './postmortem';

function report(over: Partial<PostmortemReport> = {}): PostmortemReport {
  return {
    id: 's1',
    outcome: 'incomplete',
    attempts: 3,
    maxXCol: 60,
    gates: { EXECUTE: 10, RE_SENSE: 2, ESCALATE: 1 },
    execRate: 0.8,
    decisions: 13,
    deaths: [
      { cause: 'goomba', col: 28, landmark: 'goomba-pair' },
      { cause: 'goomba', col: 28, landmark: 'goomba-pair' },
      { cause: 'pit', col: 70, landmark: 'gap-1' },
    ],
    deathCauses: [
      { cause: 'goomba', count: 2 },
      { cause: 'pit', count: 1 },
    ],
    stallSites: [{ landmark: 'pipe-h2', count: 1, maxXCol: 40 }],
    actionMix: [],
    vetoes: 1,
    timeline: [],
    hypotheses: [],
    suggestions: [],
    ...over,
  } as PostmortemReport;
}

describe('policy 信封', () => {
  it('空文件 → 默认策略,无 issue', () => {
    const { envelope, issues } = parsePolicyFile(null);
    expect(envelope.policy).toEqual(DEFAULT_POLICY);
    expect(envelope.source).toBe('default');
    expect(issues).toEqual([]);
  });

  it('旧版扁平 JSON 自动迁移为信封并 sanitize', () => {
    const raw = JSON.stringify({ intervalMs: 200, gateExecute: 0.3, gateEscalate: 0.2 });
    const { envelope, issues } = parsePolicyFile(raw);
    expect(envelope.version).toBe(1);
    expect(envelope.source).toBe('legacy');
    expect(envelope.policy.intervalMs).toBe(200);
    expect(envelope.policy.gateExecute).toBe(0.3);
    // gateEscalate 0.2 > gateExecute 0.3? 不违反(gateEscalate ≤ gateExecute 才合法),0.2≤0.3 合法
    expect(issues).toEqual([]);
  });

  it('信封 v1 解析并保留 source;内部策略仍过 sanitize(不变量钳制)', () => {
    const env = wrapPolicy({ ...DEFAULT_POLICY, gateEscalate: 0.9 }, 'commit:iter-3');
    const { envelope, issues } = parsePolicyFile(JSON.stringify(env));
    expect(envelope.source).toBe('commit:iter-3');
    // gateEscalate 0.9 > gateExecute 0.16 → 违反不变量,被钳回
    expect(envelope.policy.gateEscalate).toBeLessThanOrEqual(envelope.policy.gateExecute);
    expect(issues.length).toBeGreaterThan(0);
  });

  it('垃圾 JSON / 未知版本 → 回退默认并记录 issue', () => {
    expect(parsePolicyFile('{oops').issues.length).toBe(1);
    const { envelope, issues } = parsePolicyFile(JSON.stringify({ version: 99, policy: {} }));
    expect(envelope.policy).toEqual(DEFAULT_POLICY);
    expect(issues.length).toBe(1);
  });
});

describe('Qwen 触发器(每死必析,docs/14 §2)', () => {
  it('通关必调', () => {
    const t = shouldInvokeQwen(
      initialEvoState(),
      report({ outcome: 'win', deathCauses: [], deaths: [] }),
      'r1',
    );
    expect(t.invoke).toBe(true);
    expect(t.reason).toContain('win');
  });

  it('同一 runId 幂等:已分析过不再调(唯一去重)', () => {
    const state = { ...initialEvoState(), analyzedRunIds: ['r1'] };
    const t = shouldInvokeQwen(state, report({ outcome: 'win' }), 'r1');
    expect(t.invoke).toBe(false);
    expect(t.reason).toContain('幂等');
  });

  it('有死亡必调:新签名 death(签名),重复签名 death-repeat 标注而非去重', () => {
    const state = initialEvoState();
    const r = report();
    const t1 = shouldInvokeQwen(state, r, 'r1');
    expect(t1.invoke).toBe(true);
    expect(t1.reason).toContain('death(goomba@goomba-pair)');
    expect(deathSignature(r)).toBe('goomba@goomba-pair');
    // 签名滚动入簿记
    const s2 = recordRun(state, r, 'r1', true);
    expect(s2.lastDeathSignature).toBe('goomba@goomba-pair');
    // 同签名再次失败:仍调用,标注 repeat —— 反复失败是下局必须优化的信号
    const t2 = shouldInvokeQwen(s2, report(), 'r2');
    expect(t2.invoke).toBe(true);
    expect(t2.reason).toContain('death-repeat');
    // 换一个签名 → 回到 death(新签名)
    const t3 = shouldInvokeQwen(
      s2,
      report({
        deaths: [{ cause: 'pit', col: 70, landmark: 'gap-1' }],
        deathCauses: [{ cause: 'pit', count: 1 }],
      }),
      'r3',
    );
    expect(t3.invoke).toBe(true);
    expect(t3.reason).toContain('death(pit@gap-1)');
  });

  it('无死亡无通关:plateau 未达窗口不调(no-signal)', () => {
    const t = shouldInvokeQwen(initialEvoState(), report({ deathCauses: [], deaths: [] }), 'r1');
    expect(t.invoke).toBe(false);
    expect(t.reason).toContain('no-signal');
  });

  it('plateau:连续无进展达到窗口才触发;有进展重置', () => {
    let s = initialEvoState();
    // bestMaxXCol 起始 0;第一局 maxXCol=60 → 有进展
    s = recordRun(s, report({ deathCauses: [], deaths: [] }), 'r1', false);
    expect(s.plateauStreak).toBe(0);
    expect(s.bestMaxXCol).toBe(60);
    // 连续 3 局无进展(且无死亡签名) → 第 4 局触发 plateau
    for (const id of ['r2', 'r3', 'r4']) {
      s = recordRun(s, report({ deathCauses: [], deaths: [], maxXCol: 50 }), id, false);
    }
    expect(s.plateauStreak).toBe(3);
    const t = shouldInvokeQwen(s, report({ deathCauses: [], deaths: [], maxXCol: 50 }), 'r5');
    expect(t.invoke).toBe(true);
    expect(t.reason).toContain('plateau');
  });
});

describe('评估判定(中位数 + ε + 滞回)', () => {
  it('通关 ≫ 未通关;更快通关分更高', () => {
    const slow = runScore({ won: true, maxXCol: 212, ticks: 5000 });
    const fast = runScore({ won: true, maxXCol: 212, ticks: 1500 });
    const lose = runScore({ won: false, maxXCol: 211, ticks: 100 });
    expect(fast).toBeGreaterThan(slow);
    expect(slow).toBeGreaterThan(lose);
  });

  it('median 奇偶', () => {
    expect(median([1, 5, 9])).toBe(5);
    expect(median([1, 2, 3, 4])).toBe(2.5);
    expect(median([])).toBe(0);
    expect(
      evalMedian([
        { won: false, maxXCol: 10, ticks: 1 },
        { won: false, maxXCol: 30, ticks: 1 },
        { won: false, maxXCol: 20, ticks: 1 },
      ]),
    ).toBe(20);
  });

  it('无基线首个候选直接转正', () => {
    expect(judgeCandidate(null, 50).verdict).toBe('commit');
  });

  it('未过 +ε 阈值一律回滚(滞回吸收噪声)', () => {
    expect(judgeCandidate(100, 104.9).verdict).toBe('rollback');
    expect(judgeCandidate(100, 105.1).verdict).toBe('commit');
    expect(judgeCandidate(100, 100).verdict).toBe('rollback');
  });
});

describe('补丁纪律(限幅 + 冷却期)', () => {
  it('diffPatch 空/非空', () => {
    expect(Object.keys(diffPatch(DEFAULT_POLICY, DEFAULT_POLICY))).toEqual([]);
    const d = diffPatch(DEFAULT_POLICY, { ...DEFAULT_POLICY, vetoDistPx: 30 });
    expect(d['vetoDistPx']).toEqual({ from: 14, to: 30 });
  });

  it('数值字段步进钳到 ±20%', () => {
    const target = { ...DEFAULT_POLICY, vetoDistPx: 100 };
    const { policy, issues } = limitPatch(target, DEFAULT_POLICY);
    expect(policy.vetoDistPx).toBeCloseTo(14 * 1.2);
    expect(issues.length).toBe(1);
  });

  it('触发窗端点各钳 ±8px', () => {
    const target = { ...DEFAULT_POLICY, pipeWindow: [-40, 100] as [number, number] };
    const { policy, issues } = limitPatch(target, DEFAULT_POLICY);
    const [lo, hi] = policy.pipeWindow;
    expect(lo).toBe(DEFAULT_POLICY.pipeWindow[0] - 8);
    expect(hi).toBe(DEFAULT_POLICY.pipeWindow[1] + 8);
    expect(issues.length).toBe(1);
  });

  it('超过 3 个字段按幅度截断', () => {
    const target = {
      ...DEFAULT_POLICY,
      vetoDistPx: 20,
      gateExecute: 0.2,
      gateEscalate: 0.1,
      backoffTicks: 30,
    };
    const { policy, issues } = limitPatch(target, DEFAULT_POLICY);
    const changed = Object.keys(diffPatch(DEFAULT_POLICY, policy));
    expect(changed.length).toBe(3);
    expect(issues.some((i) => i.includes('丢弃'))).toBe(true);
  });

  it('冷却期字段被剔除,过期放行;startCooldowns 起算 M 局', () => {
    const target = { ...DEFAULT_POLICY, vetoDistPx: 20, gateExecute: 0.2 };
    const cds = startCooldowns({}, ['vetoDistPx'], 10);
    expect(cds['vetoDistPx']).toBe(10 + COOLDOWN_ITERS);
    const { policy, dropped } = applyCooldowns(target, DEFAULT_POLICY, cds, 12);
    expect(dropped).toEqual(['vetoDistPx']);
    expect(policy.vetoDistPx).toBe(DEFAULT_POLICY.vetoDistPx);
    expect(policy.gateExecute).toBe(0.2);
    // 冷却过期(iteration > until)放行
    const later = applyCooldowns(target, DEFAULT_POLICY, cds, 16);
    expect(later.dropped).toEqual([]);
    expect(later.policy.vetoDistPx).toBe(20);
  });
});

describe('审计与历史', () => {
  it('auditLine/parseAuditLines 往返;坏行跳过', () => {
    const line = auditLine({
      v: 1,
      ts: '2026-01-01T00:00:00Z',
      iter: 3,
      runId: 'r1',
      actor: 'qwen',
      action: 'commit',
      reason: '测试',
    });
    const parsed = parseAuditLines(`${line}\n{bad}\n\n`);
    expect(parsed).toHaveLength(1);
    expect(parsed[0]?.action).toBe('commit');
    expect(parseAuditLines(null)).toEqual([]);
  });

  it('history 解析容错 + 滚动截断', () => {
    expect(parseHistory('{bad')).toEqual([]);
    expect(parseHistory(null)).toEqual([]);
    let h = parseHistory(null);
    for (let i = 1; i <= 60; i += 1) {
      h = appendHistory(h, { iteration: i, won: false, maxXCol: i, ticks: i });
    }
    expect(h).toHaveLength(50);
    expect(h[0]?.iteration).toBe(11);
  });

  it('state.json 解析容错', () => {
    expect(parseEvoState('{bad').issues).toHaveLength(1);
    const { state } = parseEvoState(JSON.stringify({ iteration: 7, championScore: 42 }));
    expect(state.iteration).toBe(7);
    expect(state.championScore).toBe(42);
    expect(state.cooldowns).toEqual({});
  });
});

describe('经验沉淀 insights.md(docs/14 §6)', () => {
  it('追加成单行条目并带观测标记;空文本不追加', () => {
    const obs = obsSnapshotOf(DEFAULT_POLICY);
    let doc = appendInsight('', 3, '低空 gap 要 jump_run_right 而不是 jump_right', obs);
    expect(doc).toBe(
      '- [iter 3] 低空 gap 要 jump_run_right 而不是 jump_right (obs 16列/176px/pose:on/sub:on)',
    );
    doc = appendInsight(doc, 4, '  多行\n压一行  ', obs);
    expect(doc).toContain('- [iter 4] 多行 压一行');
    expect(appendInsight(doc, 5, '   ')).toBe(doc);
  });

  it('滚动到上限:最旧条目先丢;人保段标记对永不丢', () => {
    let doc = '';
    for (let i = 1; i <= 40; i += 1) {
      doc = appendInsight(doc, i, `经验 ${i} `.padEnd(80, 'x'));
    }
    expect(doc.length).toBeLessThanOrEqual(INSIGHTS_MAX_CHARS + 100);
    expect(doc).not.toContain('- [iter 1]');
    expect(doc).toContain('- [iter 40]');
    // 人保段
    const withHuman = appendInsight(
      `<!-- HUMAN -->\n人手写的经验,永远保留\n<!-- /HUMAN -->`,
      1,
      '机器经验',
    );
    let rolled = withHuman;
    for (let i = 2; i <= 40; i += 1) {
      rolled = appendInsight(rolled, i, `经验 ${i} `.padEnd(80, 'x'));
    }
    expect(rolled).toContain('人手写的经验,永远保留');
    expect(rolled).not.toContain('机器经验');
  });

  it('obsSnapshotOf 反映当前策略;审计行带 obs 快照可往返', () => {
    const obs = obsSnapshotOf({ ...DEFAULT_POLICY, obsProfileCols: 24, obsIncludePose: false });
    expect(obs).toEqual({ profileCols: 24, threatsLookPx: 176, pose: false, subgoal: true });
    const line = auditLine({
      v: 1,
      ts: '2026-01-01T00:00:00Z',
      iter: 3,
      runId: 'r1',
      actor: 'qwen',
      action: 'propose',
      reason: 'death(pit@gap-1)',
      obs,
    });
    expect(parseAuditLines(line)[0]?.obs).toEqual(obs);
  });
});
