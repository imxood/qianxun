import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { afterAll, describe, expect, it } from 'vitest';

import { loadState, refineWithQwen, runHeadlessSession, runLoop } from './mario-loop';
import { DEFAULT_POLICY, setPolicy } from '../src/features/games/mario/policy';

const tmp = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'qx-evo-')));
afterAll(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

describe('自主进化闭环(本地)', () => {
  it('自驾头面会话:确定性通关并产出复盘', async () => {
    setPolicy(DEFAULT_POLICY);
    const r = await runHeadlessSession({ mode: 'autopilot', maxTicks: 60 * 120 });
    expect(r.won).toBe(true);
    expect(r.report.outcome).toBe('win');
    expect(r.report.maxXCol).toBeGreaterThan(190);
  }, 30_000);

  it('runLoop:两局迭代写 policy 信封/playbook/history/state 四件套', async () => {
    const results = await runLoop({
      iterations: 2,
      mode: 'autopilot',
      dataDir: tmp,
      log: () => {},
    });
    expect(results).toHaveLength(2);
    expect(results.every((r) => r.won)).toBe(true);
    expect(fs.existsSync(path.join(tmp, 'policy.json'))).toBe(true);
    expect(fs.existsSync(path.join(tmp, 'playbook.md'))).toBe(true);
    expect(fs.existsSync(path.join(tmp, 'history.json'))).toBe(true);
    expect(fs.existsSync(path.join(tmp, 'state.json'))).toBe(true);
    const state = loadState(tmp);
    expect(state.history).toHaveLength(2);
    expect(state.envelope.version).toBe(1);
    expect(state.evo.iteration).toBe(2);
  }, 60_000);

  it('refineWithQwen:越界补丁钳制 + 未知顶层键可观测 + insight 可证伪', async () => {
    const fake = (async () => ({
      ok: true,
      json: async () => ({
        choices: [
          {
            message: {
              content:
                '{"playbook":"pipe46 必须满速助跑;gap1 提前 8px 起跳。","policy_patch":{"intervalMs":9999,"goombaWindow":[-99,88],"gateExecute":0.3},"per_tick":{"tick":1,"action":"jump"},"insight":{"kind":"claim","claim":"前扫288px 低置信减半","metric":"avgConf","direction":"up","evidenceIter":[3]}}',
            },
          },
        ],
      }),
    })) as unknown as typeof fetch;
    const r = await refineWithQwen({
      playbook: '旧手册',
      report: {
        id: 't',
        outcome: 'incomplete',
        attempts: 3,
        score: 0,
        maxXCol: 27,
        progressPct: 12,
        durationTicks: 1800,
        decisions: 100,
        gates: { exec: 20, reSense: 0, escalate: 70, stale: 10, guard: 0 },
        execRate: 0.2,
        escalateRate: 0.7,
        staleRate: 0.1,
        avgConf: 0.1,
        p50LatencyMs: 240,
        deaths: [],
        deathCauses: [{ cause: 'goomba', count: 3 }],
        stallSites: [{ landmark: 'pipe@46(h4)', count: 2, maxXCol: 45.8 }],
        thrashSites: [],
        churnSites: [],
        firstDeathTick: null,
        actionMix: [],
        vetoes: 4,
        timeline: [],
        hypotheses: [],
        suggestions: [],
      },
      policy: DEFAULT_POLICY,
      history: [],
      fetchImpl: fake,
    });
    expect(r.playbook).toContain('pipe46');
    expect(r.playbook.length).toBeLessThanOrEqual(1600);
    expect(r.policy.intervalMs).toBe(600); // 钳制
    expect(r.policy.goombaWindow).toEqual([-20, 60]); // 钳制
    expect(r.policy.gateExecute).toBe(0.3); // 合法值直接生效
    // 越权顶层键可观测(docs/15 §6.6 硬化①)
    expect(r.issues.some((x) => x.includes('越权顶层键 per_tick'))).toBe(true);
    // 可证伪 insight 解析(docs/15 §6.5)
    expect(r.insight?.metric).toBe('avgConf');
    expect(r.insight?.direction).toBe('up');
  });
});
