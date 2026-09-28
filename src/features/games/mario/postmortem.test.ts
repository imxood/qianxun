import { describe, expect, it } from 'vitest';

import { buildPostmortem, landmarkOf, renderMarkdown, type SessionInput } from './postmortem';
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
});
