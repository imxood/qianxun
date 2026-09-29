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
  diffPatch,
  detectRegression,
  evalMedian,
  initialEvoState,
  judgeCandidate,
  limitPatch,
  markAnalyzed,
  median,
  obsSnapshotOf,
  parseAuditLines,
  parseEvoState,
  parseHistory,
  parsePolicyFile,
  planAnalyses,
  reasonShort,
  recordRun,
  recordVersion,
  restoreVersion,
  runScore,
  startCooldowns,
  wrapPolicy,
  insightSimilarity,
} from './evolution';
import {
  nextTask,
  scheduleTask,
  taskFailed,
  transition,
  type LoopState,
  type SchedTask,
} from './machine';
import { DEFAULT_POLICY, sanitizePolicy } from './policy';
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
      { cause: 'goomba', col: 28, landmark: 'goomba-pair', attempt: 1 },
      { cause: 'goomba', col: 28, landmark: 'goomba-pair', attempt: 2 },
      { cause: 'pit', col: 70, landmark: 'gap-1', attempt: 3 },
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

describe('Qwen 触发器(死亡级分析清单,docs/15 §6.1)', () => {
  it('通关必调(run 级)', () => {
    const t = planAnalyses(
      initialEvoState(),
      report({ outcome: 'win', deathCauses: [], deaths: [] }),
      'r1',
    );
    expect(t).toHaveLength(1);
    expect(t[0]?.kind).toBe('win');
    expect(t[0]?.reason).toContain('win');
  });

  it('runId 幂等:win 分析过不再调', () => {
    const s0 = initialEvoState();
    const win = report({ outcome: 'win', deathCauses: [], deaths: [] });
    const task = planAnalyses(s0, win, 'r1')[0]!;
    const s1 = markAnalyzed(s0, task);
    expect(planAnalyses(s1, win, 'r1')).toHaveLength(0);
  });

  it('每死一个任务:deathKey=runId#attempt;重复签名标注 death-repeat', () => {
    const s0 = initialEvoState();
    const r = report(); // 2×goomba@28(a1,a2)+1×pit(a3)
    const t1 = planAnalyses(s0, r, 'r1');
    expect(t1.map((x) => x.key)).toEqual(['r1#1', 'r1#2', 'r1#3']);
    expect(t1[0]?.sig).toBe('goomba@goomba-pair');
    expect(t1[0]?.reason).toBe('death(goomba@goomba-pair)');
    // 死亡入统计后,第二次同签名 → repeat
    const s1 = noteDeathSigsPublic(s0, r);
    const t2 = planAnalyses(s1, r, 'r2');
    expect(t2[0]?.reason).toContain('death-repeat');
    // 成功后才占幂等键(docs/15 R6):同 runId 下,已析的 attempt 不再出任务
    const s2 = markAnalyzed(s1, t2[0]!);
    const t3 = planAnalyses(s2, r, 'r2');
    expect(t3.map((x) => x.key)).not.toContain('r2#1');
    expect(t3.map((x) => x.key)).toContain('r2#2');
  });

  it('无死亡无通关:plateau 达窗才出 run 级任务,否则无任务', () => {
    const s = initialEvoState();
    expect(planAnalyses(s, report({ deathCauses: [], deaths: [] }), 'r1')).toHaveLength(0);
    const s2 = { ...s, plateauStreak: 3 };
    const t = planAnalyses(s2, report({ deathCauses: [], deaths: [] }), 'r2');
    expect(t).toHaveLength(1);
    expect(t[0]?.kind).toBe('plateau');
  });

  it('plateau:连续无进展达到窗口才触发;有进展重置', () => {
    let s = initialEvoState();
    s = recordRun(s, report({ deathCauses: [], deaths: [] }), 'r1', false);
    expect(s.plateauStreak).toBe(0);
    expect(s.bestMaxXCol).toBe(60);
    for (const id of ['r2', 'r3', 'r4']) {
      s = recordRun(s, report({ deathCauses: [], deaths: [], maxXCol: 50 }), id, false);
    }
    expect(s.plateauStreak).toBe(3);
    const t = planAnalyses(s, report({ deathCauses: [], deaths: [], maxXCol: 50 }), 'r5');
    expect(t[0]?.kind).toBe('plateau');
  });

  it('recordRun 计数器:局数/胜场/死亡分布/模式', () => {
    let s = initialEvoState();
    s = recordRun(
      s,
      report({ outcome: 'win', deathCauses: [], deaths: [] }),
      'r1',
      false,
      'reflex',
    );
    s = recordRun(s, report(), 'r2', false, 'reflex');
    expect(s.counters.runs).toBe(2);
    expect(s.counters.wins).toBe(1);
    expect(s.counters.deaths).toBe(3);
    expect(s.counters.deathsByCause['goomba']).toBe(2);
    expect(s.counters.deathsByLandmark['goomba-pair']).toBe(2);
    expect(s.counters.perMode['reflex']).toEqual({ runs: 2, wins: 1 });
    expect(s.sigStats['goomba@goomba-pair']?.deaths).toBe(2);
  });
});

/** 测试助手:走公开 API 累积签名统计。 */
function noteDeathSigsPublic(s: ReturnType<typeof initialEvoState>, r: PostmortemReport) {
  return recordRun(s, { ...r, outcome: r.outcome, maxXCol: r.maxXCol }, 'tmp', false);
}

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

  it('history 解析容错 + 滚动截断(cap 200,docs/15 §7)', () => {
    expect(parseHistory('{bad')).toEqual([]);
    expect(parseHistory(null)).toEqual([]);
    let h = parseHistory(null);
    for (let i = 1; i <= 220; i += 1) {
      h = appendHistory(h, { iteration: i, won: false, maxXCol: i, ticks: i });
    }
    expect(h).toHaveLength(200);
    expect(h[0]?.iteration).toBe(21);
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
    // 构造语义互异的条目(LCG 伪随机填充逐条去相关,避免近重复被合并)
    const scene = (i: number): string => {
      let x = Math.imul(i + 1, 2654435761) >>> 0;
      let filler = '';
      for (let k = 0; k < 36; k += 1) {
        x = (Math.imul(x, 1664525) + 1013904223) >>> 0;
        filler += String((x % 89) + 10);
      }
      return (
        `场景${i}:col${i * 3}处${['管道', '深坑', '板牙', '楼梯', '悬崖'][i % 5]}` +
        `需提前${(i % 7) + 1}拍${['起跳', '下压', '冲刺', '急停'][i % 4]}${filler}`
      );
    };
    let doc = '';
    for (let i = 1; i <= 60; i += 1) {
      doc = appendInsight(doc, i, scene(i));
    }
    expect(doc.length).toBeLessThanOrEqual(INSIGHTS_MAX_CHARS + 100);
    expect(doc).not.toContain('- [iter 1]');
    expect(doc).toContain('- [iter 60]');
    // 人保段
    const withHuman = appendInsight(
      `<!-- HUMAN -->\n人手写的经验,永远保留\n<!-- /HUMAN -->`,
      1,
      '机器经验',
    );
    let rolled = withHuman;
    for (let i = 2; i <= 60; i += 1) {
      rolled = appendInsight(rolled, i, scene(i));
    }
    expect(rolled).toContain('人手写的经验,永远保留');
    expect(rolled).not.toContain('机器经验');
  });

  it('obsSnapshotOf 反映当前策略(含消息设计);审计行带 obs 快照可往返', () => {
    const obs = obsSnapshotOf({
      ...DEFAULT_POLICY,
      obsProfileCols: 24,
      obsIncludePose: false,
      obsThreatFormat: 'rows',
      obsStateExtra: ['stallTicks'],
    });
    expect(obs).toEqual({
      profileCols: 24,
      threatsLookPx: 176,
      pose: false,
      subgoal: true,
      threatFormat: 'rows',
      progressStyle: 'full',
      hintStyle: 'full',
      instructionVariant: 'default',
      stateExtra: 'stallTicks',
    });
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

describe('消息设计沙箱(docs/15 §6.4)', () => {
  it('枚举校验:表外值回默认 + issue;obsStateExtra 白名单/去重/≤2', () => {
    const r = sanitizePolicy({
      obsThreatFormat: 'diagonal',
      obsHintStyle: 42,
      obsStateExtra: ['stallTicks', 'stallTicks', 'hacker', 'lastAction', 'heldTicks'],
    });
    expect(r.policy.obsThreatFormat).toBe('named');
    expect(r.policy.obsHintStyle).toBe('full');
    expect(r.policy.obsStateExtra).toEqual(['stallTicks', 'lastAction']);
    expect(r.issues.length).toBe(3);
    const ok = sanitizePolicy({ obsThreatFormat: 'rows', obsInstructionVariant: 'concise' });
    expect(ok.policy.obsThreatFormat).toBe('rows');
    expect(ok.policy.obsInstructionVariant).toBe('concise');
    expect(ok.issues).toEqual([]);
  });

  it('limitPatch 单维度:数值与消息设计混车 → 保留多数维度,余丢弃 + kind', () => {
    const mixed = {
      ...DEFAULT_POLICY,
      gateExecute: 0.2,
      obsThreatFormat: 'rows' as const,
      obsHintStyle: 'terse' as const,
    };
    const r = limitPatch(mixed, DEFAULT_POLICY);
    expect(r.kind).toBe('design'); // 设计 2 字段 > 数值 1 字段
    expect(r.policy.obsThreatFormat).toBe('rows');
    expect(r.policy.gateExecute).toBe(DEFAULT_POLICY.gateExecute);
    expect(r.issues.some((x) => x.includes('混车'))).toBe(true);
    const numericOnly = limitPatch({ ...DEFAULT_POLICY, gateExecute: 0.2 }, DEFAULT_POLICY);
    expect(numericOnly.kind).toBe('numeric');
    const none = limitPatch(DEFAULT_POLICY, DEFAULT_POLICY);
    expect(none.kind).toBe('none');
  });

  it('枚举补丁进入 diffPatch 与冷却(零改动覆盖)', () => {
    const target = { ...DEFAULT_POLICY, obsThreatFormat: 'rows' as const };
    const patch = diffPatch(DEFAULT_POLICY, target);
    expect(patch['obsThreatFormat']).toEqual({ from: 'named', to: 'rows' });
    const cds = startCooldowns({}, ['obsThreatFormat'], 3);
    const { dropped } = applyCooldowns(target, DEFAULT_POLICY, cds, 4);
    expect(dropped).toEqual(['obsThreatFormat']);
  });

  it('窗口透传不产噪声:champion 同值合并 → 0 issue(反事实前提教训)', () => {
    const champion = { ...DEFAULT_POLICY, pipeWindow: [-6, 8] as [number, number] };
    const r = sanitizePolicy(champion); // 合并体原样透传
    expect(r.issues).toEqual([]);
    const bad = sanitizePolicy({ ...champion, pipeWindow: [-999, 999] as [number, number] });
    expect(bad.issues.length).toBe(1);
  });
});

describe('经验库卫生(docs/15 §9b):近重复确认 + 相似度', () => {
  it('近重复不再整条入库,原行追加 ‖确认@iter', () => {
    const obs = obsSnapshotOf(DEFAULT_POLICY);
    let doc = appendInsight('', 3, '扩大观测视野至 24 列能显著降低原地空转率', obs);
    doc = appendInsight(doc, 5, '扩大观测视野至 24 列能显著降低原地空转率', obs); // 完全同义
    expect(doc.match(/- \[iter /g)).toHaveLength(1);
    expect(doc).toContain('‖确认@5');
    // 措辞略异但归一化后高度相似 → 仍算近重复
    doc = appendInsight(doc, 6, '扩大观测视野到24列,能显著降低 原地空转率!', obs);
    expect(doc.match(/- \[iter /g)).toHaveLength(1);
    expect(doc).toContain('‖确认@6');
    // 语义不同 → 正常入库
    doc = appendInsight(doc, 7, '楼梯区提前起跳,贴墙滑行会掉', obs);
    expect(doc.match(/- \[iter /g)).toHaveLength(2);
    expect(insightSimilarity('苹果很甜', '完全无关的话题内容')).toBeLessThan(0.3);
  });
});

describe('分析调度器与状态机(docs/15 §3.1/§6.2)', () => {
  const t = (key: string, priority: number): SchedTask => ({
    key,
    kind: 'death',
    attempt: 1,
    sig: null,
    reason: `death(${key})`,
    priority,
  });

  it('scheduleTask:同 key 合并;cap 溢出弹最低优先级', () => {
    let q: SchedTask[] = [];
    q = scheduleTask(q, t('a', 1)).queue;
    const m = scheduleTask(q, t('a', 2));
    expect(m.merged).toBe(true);
    expect(m.queue).toHaveLength(1);
    expect(m.queue[0]?.priority).toBe(2); // 留最新
    let q2: SchedTask[] = [];
    for (let i = 0; i < 6; i += 1) q2 = scheduleTask(q2, t(`k${i}`, i)).queue;
    const over = scheduleTask(q2, t('low', 99));
    expect(over.overflow?.key).toBe('low');
    expect(over.queue).toHaveLength(6);
  });

  it('nextTask:backlog 优先于队列;按优先级;taskFailed 2 次后放弃', () => {
    const backlog = [t('b1', 5)];
    const queue = [t('q1', 1)];
    expect(nextTask(backlog, queue)?.key).toBe('b1');
    expect(nextTask([], queue)?.key).toBe('q1');
    expect(nextTask([], [])).toBeNull();
    const r1 = taskFailed(t('x', 1));
    expect(r1?.attempts).toBe(1);
    const r2 = taskFailed(r1!);
    expect(r2?.attempts).toBe(2);
    expect(taskFailed(r2!)).toBeNull();
  });

  it('transition:主链 + pause/resume 记原相;非法迁移保持原相', () => {
    let st: LoopState = { phase: 'IDLE' };
    st = transition(st, 'start');
    expect(st.phase).toBe('TRAINING');
    st = transition(st, 'runEnd');
    st = transition(st, 'plan');
    st = transition(st, 'enqueue');
    st = transition(st, 'dequeue');
    expect(st.phase).toBe('ANALYZING');
    st = transition(st, 'propose');
    st = transition(st, 'verdict');
    st = transition(st, 'beginRun');
    expect(st.phase).toBe('TRAINING');
    const p = transition(st, 'pause');
    expect(p.phase).toBe('PAUSED');
    expect(transition(p, 'resume').phase).toBe('TRAINING');
    expect(transition(st, 'verdict').phase).toBe('TRAINING'); // 非法迁移不生效
  });

  it('reasonShort:两层文案不互渗', () => {
    expect(reasonShort('already-analyzed(同一 runId 幂等)')).toBe('已析过');
    expect(reasonShort('death(pit@gap-1)')).toBe('死亡:pit@gap-1');
    expect(reasonShort('death-repeat(goomba@goomba-pair)')).toBe('重复死亡:goomba@goomba-pair');
    expect(reasonShort('win(沉淀成功经验)')).toBe('通关沉淀');
    expect(reasonShort('plateau(连续 3 局无进展)')).toBe('平台期3局');
  });

  it('版本账本与回归守卫:recordVersion/restoreVersion/detectRegression', () => {
    let s = initialEvoState();
    s = recordVersion(s, {
      policy: { ...DEFAULT_POLICY, vetoDistPx: 20 },
      patch: {},
      originSigs: [],
      verdict: 'commit',
      score: 100,
    });
    expect(s.championVid).toBe(1);
    const r = restoreVersion(s, 1);
    expect(r?.policy.vetoDistPx).toBe(20);
    expect(r?.state.championScore).toBeNull(); // 强制重测
    expect(restoreVersion(s, 99)).toBeNull();
    // 回归:20 局,前 10 局 80% 胜率,近 10 局 20% → 触发
    const h = Array.from({ length: 20 }, (_, i) => ({
      iteration: i + 1,
      won: i < 8 || i >= 18,
      maxXCol: 50,
      ticks: 1000,
    }));
    const reg = detectRegression(h);
    expect(reg).not.toBeNull();
    expect(reg?.current).toBeLessThan(0.5);
  });
});
