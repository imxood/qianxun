import { describe, expect, it } from 'vitest';

import { DEFAULT_POLICY, sanitizePolicy, setPolicy, getPolicy } from './policy';

describe('policy 策略沙箱', () => {
  it('越界钳制并报告 issues', () => {
    const { policy, issues } = sanitizePolicy({
      intervalMs: 9999,
      gateExecute: 0.001,
      vetoDistPx: 500,
    });
    expect(policy.intervalMs).toBe(600);
    expect(policy.gateExecute).toBe(0.05); // 0.001 → 0.05(下限,尊重设定)
    expect(policy.gateEscalate).toBe(0.05); // 不变式:ESC 压到 ≤ EXEC
    expect(policy.vetoDistPx).toBe(40);
    expect(issues.length).toBe(4); // 3 次钳制 + 1 次不变式
    expect(issues[0]).toContain('钳制');
  });

  it('窗口:逆序交换 + 值域钳制;未知字段丢弃', () => {
    const { policy, issues } = sanitizePolicy({
      gapWindow: [20, -25],
      goombaWindow: [-999, 999],
      hackerField: 'nope',
    });
    expect(policy.gapWindow).toEqual([-25, 20]);
    expect(policy.goombaWindow).toEqual([-20, 60]);
    expect((policy as unknown as Record<string, unknown>).hackerField).toBeUndefined();
    expect(issues.length).toBe(2);
  });

  it('不变式:gateEscalate > gateExecute 自动下压', () => {
    const { policy } = sanitizePolicy({ gateExecute: 0.1, gateEscalate: 0.3 });
    expect(policy.gateExecute).toBeGreaterThanOrEqual(policy.gateEscalate);
    expect(policy.gateExecute).toBe(0.1); // 用户设定不被改动
    expect(policy.gateEscalate).toBe(0.1);
    // 单独压低 gateExecute 会与默认 gateEscalate(0.12)撞线 → ESC 跟随下压
    const r2 = sanitizePolicy({ gateExecute: 0.001 });
    expect(r2.policy.gateExecute).toBe(0.05);
    expect(r2.policy.gateEscalate).toBe(0.05);
  });

  it('非对象输入 → 原样默认策略', () => {
    const { policy, issues } = sanitizePolicy('junk');
    expect(policy).toEqual(DEFAULT_POLICY);
    expect(issues).toHaveLength(0);
  });

  it('setPolicy 热生效(规划器/驱动运行时读取)', () => {
    const p = { ...DEFAULT_POLICY, vetoDistPx: 30, intervalMs: 400 };
    setPolicy(p);
    expect(getPolicy().vetoDistPx).toBe(30);
    setPolicy(DEFAULT_POLICY);
    expect(getPolicy().vetoDistPx).toBe(14);
  });
});
