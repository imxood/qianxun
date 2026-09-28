import { describe, expect, it } from 'vitest';

import {
  buildPostmortem,
  landmarkOf,
  processDigest,
  renderMarkdown,
  type SessionInput,
} from './postmortem';
import { TILE } from './world1-1';

function session(over: Partial<SessionInput> = {}): SessionInput {
  return {
    id: 't1',
    mode: 'reflex',
    decisions: [
      { action: 'run_right', conf: 0.5, gate: 'EXECUTE', latencyMs: 200, applied: true },
      { action: 'idle', conf: 0.05, gate: 'ESCALATE', latencyMs: 250, applied: true },
      {
        action: 'jump_right',
        conf: 0.4,
        gate: 'EXECUTE',
        latencyMs: 220,
        applied: false,
        note: 'stale',
      },
    ],
    events: [
      { type: 'death', cause: 'goomba', x: 28 * TILE + 4, attempt: 1 },
      { type: 'death', cause: 'pit', x: 69 * TILE + 8, attempt: 2 },
      { type: 'stall', x: 46 * TILE - 10, maxX: 46 * TILE - 4, tick: 600 },
      { type: 'win', score: 1300, attempt: 3 },
    ],
    samples: [
      { tick: 60, maxX: 16 * TILE },
      { tick: 600, maxX: 47 * TILE },
    ],
    final: { attempts: 3, score: 1300, coins: 4, maxX: 47 * TILE, phase: 'running' },
    vetoes: 6,
    ...over,
  };
}

describe('postmortem 复盘引擎', () => {
  it('地标命名:管道/坑/楼梯/旗杆', () => {
    expect(landmarkOf(28)).toBe('pipe@28(h2)');
    expect(landmarkOf(46)).toBe('pipe@46(h4)');
    expect(landmarkOf(69)).toBe('gap@69-70');
    expect(landmarkOf(198)).toBe('flag@198');
    expect(landmarkOf(10)).toBe('col10');
  });

  it('统计:stale 不计入门控比例,死因/停滞按地标聚合', () => {
    const r = buildPostmortem(session());
    expect(r.decisions).toBe(3);
    expect(r.gates.exec).toBe(1); // 第三拍 applied=false → 只进 stale
    expect(r.gates.stale).toBe(1);
    expect(r.gates.escalate).toBe(1);
    expect(r.execRate).toBeCloseTo(0.33, 1);
    expect(r.staleRate).toBeCloseTo(0.33, 1);
    expect(r.avgConf).toBeCloseTo(0.275, 2); // 只统计 applied 的置信度
    expect(r.p50LatencyMs).toBe(250); // applied 延迟上中位
    expect(r.deaths).toHaveLength(2);
    expect(r.deaths[0]).toMatchObject({ cause: 'goomba', landmark: 'pipe@28(h2)' });
    expect(r.stallSites[0]).toMatchObject({ landmark: 'pipe@46(h4)', count: 1 });
    expect(r.outcome).toBe('win');
    expect(r.attempts).toBe(3);
    expect(r.maxXCol).toBeCloseTo(47, 1);
    expect(r.vetoes).toBe(6);
  });

  it('规则假设:veto/stale/通关不出失败密度假设', () => {
    const r = buildPostmortem(session());
    expect(r.hypotheses.some((x) => x.includes('本能否决 6 次'))).toBe(true);
    expect(r.suggestions.some((x) => x.includes('veto 率'))).toBe(true);
    // stale 1/3 = 0.33 > 0.1 → 触发延迟假设
    expect(r.hypotheses.some((x) => x.includes('stale 率'))).toBe(true);
    // 通关 + 尝试 3 次:maxXCol 47 < 60 会误报失败密度 → outcome=win 时不应触发
    expect(r.hypotheses.some((x) => x.includes('失败密度过高'))).toBe(false);
  });

  it('未蒸馏场景:高 ESCALATE 率触发域蒸馏假设', () => {
    const r = buildPostmortem(
      session({
        decisions: Array.from({ length: 10 }, () => ({
          action: 'idle',
          conf: 0.05,
          gate: 'ESCALATE',
          latencyMs: 240,
          applied: true,
        })),
        events: [{ type: 'death', cause: 'goomba', x: 22 * TILE, attempt: 1 }],
        vetoes: 0,
      }),
    );
    expect(r.escalateRate).toBeGreaterThan(0.5);
    expect(r.hypotheses.some((x) => x.includes('域未蒸馏'))).toBe(true);
    expect(r.suggestions.some((x) => x.includes('教师轨迹'))).toBe(true);
  });

  it('Markdown:固定小节齐全,内嵌 JSON 可解析', () => {
    const md = renderMarkdown(buildPostmortem(session()));
    for (const sec of [
      '# Laya Jump 复盘 · t1',
      '## 失败分布',
      '## 死亡明细',
      '## 动作混合 Top5',
      '## 时间线',
      '## 根因假设(规则判定)',
      '## 下一步实验建议',
      '## 原始统计(JSON)',
    ]) {
      expect(md).toContain(sec);
    }
    const jsonLine = md.split('```json')[1]!.split('```')[0]!.trim();
    const parsed = JSON.parse(jsonLine) as { decisions: number };
    expect(parsed.decisions).toBe(3);
  });

  it('首死 tick + 死前上下文:<120 tick 触发"直接掉崖/撞兵"假设(docs/14 §3.1)', () => {
    const ctx = [
      { action: 'run_right', conf: 0.4, gate: 'EXECUTE', col: 20 },
      { action: 'run_right', conf: 0.3, gate: 'EXECUTE', col: 21 },
    ];
    const r = buildPostmortem(
      session({
        events: [
          { type: 'death', cause: 'goomba', x: 22 * TILE, attempt: 1, tick: 90, context: ctx },
        ],
      }),
    );
    expect(r.firstDeathTick).toBe(90);
    expect(r.deaths[0]?.tick).toBe(90);
    expect(r.deaths[0]?.context).toEqual(ctx);
    expect(r.hypotheses.some((x) => x.includes('直接掉崖/撞兵'))).toBe(true);
    const md = renderMarkdown(r);
    expect(md).toContain('过程异常');
    expect(md).toContain('首死 @ tick 90');
  });

  it('门控抖动:同 col 桶 EXECUTE↔ESCALATE 往返 ≥3 记为热区(docs/14 §3.2)', () => {
    // col 40 桶(±2):4 次往返;col 80 桶只 1 次 → 不上榜
    const seq = [
      ...Array.from({ length: 9 }, (_, i) => ({
        action: 'run_right',
        conf: 0.3,
        gate: i % 2 === 0 ? 'EXECUTE' : 'ESCALATE',
        latencyMs: 200,
        applied: true,
        col: 40,
      })),
      { action: 'run_right', conf: 0.5, gate: 'EXECUTE', latencyMs: 200, applied: true, col: 80 },
      { action: 'run_right', conf: 0.05, gate: 'ESCALATE', latencyMs: 200, applied: true, col: 80 },
    ];
    const r = buildPostmortem(session({ decisions: seq, events: [] }));
    expect(r.thrashSites).toEqual([{ col: 40, count: 8 }]);
    // 兜底拍(stall-guard)的 ESCALATE 不算抖动
    const guarded = buildPostmortem(
      session({
        decisions: seq.map((d) => ({ ...d, note: 'stall-guard' })),
        events: [],
      }),
    );
    expect(guarded.thrashSites).toEqual([]);
  });

  it('原地空转:同 col 桶同动作连续 ≥4 拍记为热区', () => {
    const seq = [
      ...Array.from({ length: 5 }, () => ({
        action: 'jump_right',
        conf: 0.5,
        gate: 'EXECUTE',
        latencyMs: 200,
        applied: true,
        col: 28,
      })),
      { action: 'run_right', conf: 0.5, gate: 'EXECUTE', latencyMs: 200, applied: true, col: 28 },
      ...Array.from({ length: 3 }, () => ({
        action: 'idle',
        conf: 0.5,
        gate: 'EXECUTE',
        latencyMs: 200,
        applied: true,
        col: 30,
      })),
    ];
    const r = buildPostmortem(session({ decisions: seq, events: [] }));
    expect(r.churnSites).toEqual([{ col: 28, action: 'jump_right', count: 5 }]);
  });

  it('processDigest:紧凑过程摘要含死亡上下文/异常热区/门控分布', () => {
    const r = buildPostmortem(
      session({
        events: [
          {
            type: 'death',
            cause: 'pit',
            x: 69 * TILE,
            attempt: 1,
            tick: 300,
            context: [{ action: 'run_right', conf: 0.2, gate: 'EXECUTE', col: 68 }],
          },
        ],
      }),
    );
    const d = processDigest(r) as {
      结果: string;
      首死tick: number;
      死亡: Array<{ 死因: string; 死前: string[] }>;
      门控: { EXEC率: number };
    };
    expect(d.结果).toBe('incomplete');
    expect(d.首死tick).toBe(300);
    expect(d.死亡[0]?.死因).toBe('pit');
    expect(d.死亡[0]?.死前[0]).toContain('run_right@col68');
    expect(d.门控.EXEC率).toBeGreaterThan(0);
  });
});
