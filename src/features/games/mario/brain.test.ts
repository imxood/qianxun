import { describe, expect, it } from 'vitest';

import {
  applyGate,
  checkEndpoint,
  encodeState,
  MarioBrain,
  GATE_EXECUTE,
  GATE_ESCALATE,
} from './brain';
import { createGameState } from './engine';
import { WORLD_1_1, TILE } from './world1-1';

type FetchMock = (url: string) => Promise<{ ok: boolean; json: () => Promise<unknown> }>;

function brainWith(
  responses: Array<{ conf: number; choice: string; probs?: Record<string, number> }>,
): {
  brain: MarioBrain;
  calls: () => number;
} {
  let n = 0;
  const fetchImpl: FetchMock = () => {
    const r = responses[Math.min(n, responses.length - 1)]!;
    n += 1;
    return Promise.resolve({
      ok: true,
      json: () =>
        Promise.resolve({
          answers: { action: { choice: r.choice, confidence: r.conf, probabilities: r.probs } },
        }),
    });
  };
  return {
    brain: new MarioBrain({ fetchImpl: fetchImpl as unknown as typeof fetch }),
    calls: () => n,
  };
}

describe('mario brain', () => {
  it('门控三分支阈值', () => {
    expect(applyGate(0.9, GATE_EXECUTE, GATE_ESCALATE)).toBe('EXECUTE');
    expect(applyGate(0.14, GATE_EXECUTE, GATE_ESCALATE)).toBe('RE_SENSE');
    expect(applyGate(0.05, GATE_EXECUTE, GATE_ESCALATE)).toBe('ESCALATE');
  });

  it('编码:模板版本 + profile16 + subgoal(reflex 有/pure 无)', () => {
    const s = createGameState();
    const reflex = encodeState(
      s,
      WORLD_1_1,
      { type: 'clear', hazard: null, candidates: ['run_right'], note: 'clear' },
      'reflex',
    );
    expect(reflex.encoding).toBe('v1-profile');
    expect(String(reflex.subgoal)).toContain('clear');
    const pure = encodeState(s, WORLD_1_1, null, 'pure');
    expect(String(pure.subgoal)).toContain('pure reflex');
    expect((reflex.profile16 as string).split(' ')).toHaveLength(16);
  });

  it('decide:集外 choice 概率回退到候选;EXECUTE 直接采用', async () => {
    const { brain } = brainWith([
      { conf: 0.6, choice: 'idle', probs: { jump_run_right: 0.3, jump_right: 0.1 } },
    ]);
    brain.mode = 'reflex';
    const s = createGameState();
    s.mario.x = 69 * TILE - 40;
    const d = await brain.decide(s, WORLD_1_1);
    // 'idle' 不在 gap 候选集内 → 按候选上的概率回退:jump_run_right(0.3) 胜出
    expect(d.action).toBe('jump_run_right');
    expect(d.gate).toBe('EXECUTE');
    expect(d.candidates).toContain('jump_run_right');
  });

  it('decide:RE_SENSE 再感知一次(共 2 次请求)', async () => {
    const { brain, calls } = brainWith([
      { conf: 0.13, choice: 'run_right' },
      { conf: 0.4, choice: 'run_right' },
    ]);
    brain.mode = 'reflex';
    const s = createGameState();
    const d = await brain.decide(s, WORLD_1_1);
    expect(calls()).toBe(2);
    expect(d.gate).toBe('EXECUTE');
    expect(d.sensed).toBe(2);
  });

  it('decide:低置信 ESCALATE → 规划器接管机动', async () => {
    const { brain } = brainWith([{ conf: 0.05, choice: 'idle' }]);
    brain.mode = 'reflex';
    const s = createGameState();
    const d = await brain.decide(s, WORLD_1_1);
    expect(d.gate).toBe('ESCALATE');
    expect(d.escalateManeuver).toBe(true);
  });

  it('checkEndpoint:探活成功/失败', async () => {
    expect(
      await checkEndpoint('http://x/health', (async () => ({
        ok: true,
      })) as unknown as typeof fetch),
    ).toBe(true);
    expect(
      await checkEndpoint('http://x/health', (async () => {
        throw new Error('down');
      }) as unknown as typeof fetch),
    ).toBe(false);
  });
});
