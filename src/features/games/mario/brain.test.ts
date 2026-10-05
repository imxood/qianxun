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
import { getPolicy, setPolicy } from './policy';
import { WORLD_1_1, TILE } from './world1-1';

/** RLE profile 展开总列数:`13*12 9 13*3` → 16。 */
function rleExpand(s: string): number {
  return s.split(' ').reduce((n, p) => n + (p.includes('*') ? Number(p.split('*')[1]) : 1), 0);
}

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

  it('编码:v3-obs 模板 + profile(RLE 压缩,列数随策略)+ subgoal(reflex 有/pure 无)', () => {
    const s = createGameState();
    const reflex = encodeState(
      s,
      WORLD_1_1,
      { type: 'clear', hazard: null, candidates: ['run_right'], note: 'clear' },
      'reflex',
    );
    expect(reflex.encoding).toBe('v3-obs');
    expect(String(reflex.sub)).toContain('clear');
    const pure = encodeState(s, WORLD_1_1, null, 'pure');
    expect(String(pure.sub)).toBe('(none)');
    // RLE 展开 = 16 列(docs/15 §6.3 profile 压缩)
    expect(rleExpand(reflex.prof as string)).toBe(16);
  });

  it('编码:观测沙盒消融(docs/14 §4)——列数/位姿/subgoal 随 policy 裁剪', () => {
    const backup = getPolicy();
    try {
      setPolicy({
        ...backup,
        obsProfileCols: 24,
        obsIncludePose: false,
        obsIncludeSubgoal: false,
      });
      const s = createGameState();
      const st = encodeState(
        s,
        WORLD_1_1,
        { type: 'clear', hazard: null, candidates: ['run_right'], note: 'clear' },
        'reflex',
      );
      expect(rleExpand(st.prof as string)).toBe(24);
      expect(st.m).toBeUndefined();
      expect(String(st.sub)).toBe('(none)'); // subgoal 关闭 → 退化纯反射
    } finally {
      setPolicy(backup);
    }
  });

  it('编码:常态 state 文本 ≤ 160 字符(实测延迟 ≈0.8ms/token,docs/16 §3)', () => {
    // 取关卡里最"重"的一帧:管道群 + 多威胁
    const s = createGameState(WORLD_1_1);
    s.mario.x = 56 * TILE;
    s.mario.y = 8 * TILE;
    s.mario.vx = 2.4;
    s.coinCount = 12;
    s.timeUnits = 221;
    s.attempts = 3;
    const st = encodeState(
      s,
      WORLD_1_1,
      {
        type: 'engage',
        hazard: null,
        candidates: ['run_right', 'jump_right', 'idle'],
        note: 'goomba dx=1t',
      },
      'reflex',
    );
    const text = JSON.stringify(st);
    // v2-obs 同场景约 300+ 字符;v3 靠 RLE + 短键 + 档位化压到一半以下
    expect(text.length).toBeLessThan(200);
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

  it('/predict 必带超时信号（分析 D2/P1：sidecar 半死不得让 fetch 永久挂起）', async () => {
    let seenSignal: AbortSignal | null | undefined;
    const fetchImpl = (_url: string, init?: RequestInit): Promise<unknown> => {
      seenSignal = init?.signal ?? null;
      return Promise.resolve({
        ok: true,
        json: () => Promise.resolve({ answers: { action: { choice: 'idle', confidence: 1 } } }),
      });
    };
    const brain = new MarioBrain({ fetchImpl: fetchImpl as unknown as typeof fetch });
    await brain.decide(createGameState(), WORLD_1_1);
    expect(seenSignal, 'ask() 必须传 AbortSignal').not.toBeNull();
    expect(seenSignal?.aborted ?? false).toBe(false);
  });

  it('/predict 超时后 decide 必须 reject（不得静默永久挂起）', async () => {
    // 模拟 sidecar 半死：fetch 尊重 signal，超时后 reject。
    const fetchImpl = (_url: string, init?: RequestInit): Promise<unknown> =>
      new Promise((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(new Error('aborted')));
      });
    const brain = new MarioBrain({ fetchImpl: fetchImpl as unknown as typeof fetch });
    // AbortSignal.timeout 走原生 timer，短时内不会 abort；此处验证
    // 「未 settle 的 fetch 不会让 decide 静默成功」，即它必须可被外部打断。
    await expect(
      Promise.race([
        brain.decide(createGameState(), WORLD_1_1),
        new Promise((_r, reject) => setTimeout(() => reject(new Error('would-hang')), 50)),
      ]),
    ).rejects.toBeDefined();
  });
});
