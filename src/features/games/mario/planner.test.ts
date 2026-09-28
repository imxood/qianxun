import { describe, expect, it } from 'vitest';

import { createGameState } from './engine';
import { ACTIONS, actionInput, nextHazards, nextIntent, autopilotInput } from './planner';
import { TILE, WORLD_1_1 } from './world1-1';
import { profile16, buildQuestions } from './brain';

describe('mario planner', () => {
  it('出生点前方无危险 → clear 意图', () => {
    const s = createGameState();
    const intent = nextIntent(s, WORLD_1_1);
    expect(intent.type).toBe('clear');
    expect(intent.candidates).toContain('run_right');
  });

  it('管道近前 → 提交跳跃候选;远 → 跑动候选', () => {
    const s = createGameState();
    s.goombas[0]!.alive = false; // 移开挡在 pipe28 前的 goomba(22)
    s.mario.x = 28 * TILE - 14; // 紧贴 pipe28
    const near = nextIntent(s, WORLD_1_1);
    expect(near.type).toBe('hop_pipe');
    expect(near.candidates).toContain('jump_right');
    s.mario.x = 28 * TILE - 120;
    const far = nextIntent(s, WORLD_1_1);
    expect(far.type).toBe('hop_pipe');
    expect(far.candidates).toContain('run_right');
  });

  it('坑近前 → jump_gap 提交 jump_run_right', () => {
    const s = createGameState();
    s.mario.x = 69 * TILE - 40; // gap1(69-70)前
    const intent = nextIntent(s, WORLD_1_1);
    expect(intent.type).toBe('jump_gap');
    expect(intent.candidates).toContain('jump_run_right');
  });

  it('nextHazards 按距离排序且不含身后危险', () => {
    const s = createGameState();
    s.mario.x = 60 * TILE;
    const hs = nextHazards(s, WORLD_1_1);
    expect(hs.length).toBeGreaterThan(0);
    for (let i = 1; i < hs.length; i += 1) {
      expect(hs[i]!.startX).toBeGreaterThanOrEqual(hs[i - 1]!.startX);
    }
    // 管道 28/38/46 都在身后,不应出现
    expect(hs.every((h) => h.startX >= 60 * TILE - 8)).toBe(true);
  });

  it('动作 → 输入向量映射', () => {
    expect(actionInput('jump_run_right')).toEqual({
      left: false,
      right: true,
      run: true,
      jump: true,
    });
    expect(actionInput('idle')).toEqual({
      left: false,
      right: false,
      run: false,
      jump: false,
    });
    expect(ACTIONS).toHaveLength(10);
  });

  it('autopilot:坑前触发跳跃、开阔地全速跑', () => {
    const s = createGameState();
    s.mario.x = 69 * TILE - 12; // 坑缘(已助跑状态)
    s.mario.vx = 2.4;
    expect(autopilotInput(s, WORLD_1_1).jump).toBe(true);
    const s2 = createGameState();
    s2.mario.x = 30 * TILE; // 平地(管道 28 已过,下一个危险 38 还远)
    const input = autopilotInput(s2, WORLD_1_1);
    expect(input.right).toBe(true);
    expect(input.run).toBe(true);
    expect(input.jump).toBe(false);
  });

  it('profile16:地面=13,?块列=9,坑列=15', () => {
    const s = createGameState();
    s.mario.x = 0; // base col 1 → 覆盖 col 16(索引 15? base=1 → cols 1..16,idx15=col16)
    const p = profile16(s);
    expect(p).toHaveLength(16);
    expect(p[15]).toBe(9); // col 16 的 ? 块(row 9)
    expect(p[0]).toBe(13); // 纯地面列
    const s2 = createGameState();
    s2.mario.x = 66 * TILE; // 前方是 gap1(69-70):base col 67 → idx3=col70
    const p2 = profile16(s2);
    expect(p2[3]).toBe(15); // col 70 无实心
    expect(p2[5]).toBe(13); // col 72 恢复地面
  });

  it('questions 只含候选集 criteria', () => {
    const q = buildQuestions(['jump_run_right', 'jump_right'] as const);
    const action = q.action as { criteria: Record<string, string> };
    expect(Object.keys(action.criteria).sort()).toEqual(['jump_right', 'jump_run_right']);
  });
});
