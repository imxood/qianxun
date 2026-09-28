import { describe, expect, it } from 'vitest';

import { MarioDriver } from './driver';
import { createGameState } from './engine';
import { TILE, WORLD_1_1 } from './world1-1';

describe('mario driver 护栏', () => {
  it('跳跃弧补全:上一拍 jump 且仍在上升 → 本拍延续;过顶/松键即断', () => {
    const d = new MarioDriver();
    d.configure({ mode: 'reflex' });
    const s = createGameState(WORLD_1_1);
    // 初始化 lastJump
    d.held = { left: false, right: true, run: false, jump: false };
    d.inputFor(s, WORLD_1_1);

    // 拍 A 带跳、拍 B 决策换成 run_right(无跳),但仍在上升 → 延续 jump
    d.held = { left: false, right: true, run: false, jump: true };
    s.mario.vy = -3;
    expect(d.inputFor(s, WORLD_1_1).jump).toBe(true);
    d.held = { left: false, right: true, run: true, jump: false };
    s.mario.vy = -2;
    expect(d.inputFor(s, WORLD_1_1).jump).toBe(true);
    // 过顶(vy>0)→ 不再补,尊重新拍决策
    d.held = { left: false, right: true, run: true, jump: false };
    s.mario.vy = 2;
    expect(d.inputFor(s, WORLD_1_1).jump).toBe(false);
    // 落地静止也不补
    d.held = { left: false, right: true, run: false, jump: false };
    s.mario.vy = 0;
    expect(d.inputFor(s, WORLD_1_1).jump).toBe(false);
  });

  it('human 模式不做任何补全', () => {
    const d = new MarioDriver();
    d.configure({ mode: 'human' });
    const s = createGameState(WORLD_1_1);
    expect(d.inputFor(s, WORLD_1_1)).toEqual({
      left: false,
      right: false,
      run: false,
      jump: false,
    });
  });

  it('停滞 6s → STALL 日志/事件 + 强制兜底拍(期间不请求 laya)', async () => {
    let fetches = 0;
    const fetchMock = (async () => {
      fetches += 1;
      throw new Error('laya should not be called');
    }) as unknown as typeof fetch;
    const d = new MarioDriver({ fetchImpl: fetchMock });
    d.configure({ mode: 'reflex', intervalMs: 250 });
    d.layaReady = true;
    const s = createGameState(WORLD_1_1);
    s.mario.x = 28 * TILE - 16; // 贴 pipe28
    const logs: string[] = [];
    d.onLog = (r) => logs.push(r.text);

    // 400 ticks 原地(maxX 不动;首拍消耗在 lastMaxX 初始化上,360 触发)
    for (let i = 0; i < 400; i += 1) {
      s.tick += 1;
      d.inputFor(s, WORLD_1_1);
    }
    expect(logs.some((t) => t.includes('停滞 6s'))).toBe(true);

    // 兜底拍:每 250ms 一拍,期间 laya 零请求,escalate 计数增长
    for (let b = 0; b < 12; b += 1) {
      await d.maybeDecide(s, WORLD_1_1, 1000 + b * 300);
    }
    expect(fetches).toBe(0);
    expect(d.stats.escalate).toBe(12);
    // 兜底结束后恢复请求 laya(此处 mock 抛错 → layaReady=false,fetches=1)
    await d.maybeDecide(s, WORLD_1_1, 1000 + 12 * 300);
    expect(fetches).toBe(1);
    expect(d.layaReady).toBe(false);
  });
});
