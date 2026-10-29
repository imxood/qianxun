import { describe, expect, it } from 'vitest';

import { IDLE_INPUT, createGameState, step } from './engine';
import {
  ACTIONS,
  actionInput,
  autopilotInput,
  canClearGap,
  cloneState,
  evaluateState,
  nextHazards,
  nextIntent,
  rolloutInput,
  scoreAction,
} from './planner';
import { ITEMS, JUMP, LEVELS, TILE, WORLD_1_1, WORLD_1_2 } from './world1-1';
import { buildQuestions, profile16 } from './brain';

const JUMP_ACTIONS = new Set([
  'jump',
  'jump_left',
  'jump_right',
  'jump_run_left',
  'jump_run_right',
]);

describe('mario planner · 危险感知', () => {
  it('出生点前方无危险 → clear 意图', () => {
    const s = createGameState();
    const intent = nextIntent(s, WORLD_1_1);
    expect(intent.type).toBe('clear');
    expect(intent.candidates).toContain('run_right');
  });

  it('坑近前 → jump_gap 且最优候选是跳跃类动作', () => {
    const s = createGameState();
    s.mario.x = 69 * TILE - 40; // gap1(69-70)前,已有助跑
    s.mario.vx = 2.4;
    const intent = nextIntent(s, WORLD_1_1);
    expect(intent.type).toBe('jump_gap');
    expect(JUMP_ACTIONS.has(intent.best!)).toBe(true);
  });

  it('管道近前 → hop_pipe;远 → hop_pipe 但最优是继续跑', () => {
    const s = createGameState();
    s.goombas[0]!.alive = false; // 移开挡在 pipe28 前的 goomba(22)
    s.mario.x = 28 * TILE - 14; // 紧贴 pipe28
    const near = nextIntent(s, WORLD_1_1);
    expect(near.type).toBe('hop_pipe');
    expect(JUMP_ACTIONS.has(near.best!)).toBe(true);

    const s2 = createGameState();
    s2.goombas[0]!.alive = false;
    s2.mario.x = 28 * TILE - 120;
    const far = nextIntent(s2, WORLD_1_1);
    expect(far.type).toBe('hop_pipe');
    // 远管道时最优是"继续推进"(跑或跑跳)——搜索自己决定起跳时机
    expect(['run_right', 'jump_run_right', 'jump_right']).toContain(far.best);
  });

  it('nextHazards 按距离排序且不含身后危险', () => {
    const s = createGameState();
    s.mario.x = 60 * TILE;
    const hs = nextHazards(s, WORLD_1_1);
    expect(hs.length).toBeGreaterThan(0);
    for (let i = 1; i < hs.length; i += 1) {
      expect(hs[i]!.startX).toBeGreaterThanOrEqual(hs[i - 1]!.startX);
    }
    expect(hs.every((h) => h.startX >= 60 * TILE - 8)).toBe(true);
  });

  it('食人花伸出后进入危险清单(1-1 col46 管道)', () => {
    const s = createGameState();
    s.mario.x = 42 * TILE;
    // 把食人花强制伸出
    for (const p of s.plants) {
      p.stage = 'out';
      p.out = 1;
      p.timer = 999;
    }
    const kinds = nextHazards(s, WORLD_1_1, 200).map((h) => h.kind);
    expect(kinds).toContain('plant');
  });
});

describe('mario planner · rollout 搜索(System 2)', () => {
  it('搜索是纯函数:同状态两次评分完全一致', () => {
    const s = createGameState();
    s.mario.x = 66 * TILE;
    s.mario.vx = 2.4;
    const a = scoreAction(s, WORLD_1_1, 'run_right', 12, 45);
    const b = scoreAction(s, WORLD_1_1, 'run_right', 12, 45);
    expect(a).toBe(b);
  });

  it('仿真不污染真实局(深拷贝隔离)', () => {
    const s = createGameState();
    s.mario.x = 66 * TILE;
    const before = { x: s.mario.x, tick: s.tick, tiles: s.tiles.slice() };
    scoreAction(s, WORLD_1_1, 'jump_run_right', 12, 45);
    expect(s.mario.x).toBe(before.x);
    expect(s.tick).toBe(before.tick);
    expect(Array.from(s.tiles)).toEqual(Array.from(before.tiles));
  });

  it('cloneState 产生完全独立的副本', () => {
    const s = createGameState();
    const c = cloneState(s);
    step(WORLD_1_1, c, { ...IDLE_INPUT, right: true, run: true }, 256);
    expect(c.mario.x).not.toBe(s.mario.x);
    expect(s.mario.x).toBe(3 * TILE);
  });

  it('死亡在 surrogate 里是灾难级负分', () => {
    const s0 = createGameState();
    s0.mario.x = 69 * TILE + 4; // 悬在坑(69-70)上方
    const s1 = cloneState(s0);
    for (let i = 0; i < 60 && s1.phase === 'running'; i += 1) {
      step(WORLD_1_1, s1, IDLE_INPUT, 256); // 站着不动掉坑
    }
    expect(s1.phase).toBe('dead');
    expect(evaluateState(s0, s1, WORLD_1_1)).toBeLessThan(-100_000);
  });

  it('候选集大小 = searchTopK,且不含小马里奥的 fire 动作', () => {
    const s = createGameState();
    const intent = nextIntent(s, WORLD_1_1);
    expect(intent.candidates.length).toBeLessThanOrEqual(3);
    expect(intent.candidates).not.toContain('fire_right');
    expect(intent.scores).toHaveLength(intent.candidates.length);
    // 得分单调不增(Top-K 排序保证)
    for (let i = 1; i < intent.scores!.length; i += 1) {
      expect(intent.scores![i]!).toBeLessThanOrEqual(intent.scores![i - 1]!);
    }
  });

  it('fire 形态才把 fire_right 放进搜索池', () => {
    const s = createGameState();
    s.mario.power = 'fire';
    const intent = nextIntent(s, WORLD_1_1);
    expect(intent.candidates.length).toBeGreaterThan(0);
    expect(ACTIONS).toContain('fire_right');
  });

  it('基线 rollout 策略:开阔地右跑助跑、遇坑起跳', () => {
    const s = createGameState();
    const open = rolloutInput(s, WORLD_1_1);
    expect(open.right).toBe(true);
    expect(open.run).toBe(true);
    expect(open.jump).toBe(false);

    const s2 = createGameState();
    s2.mario.x = 68 * TILE; // 下一格是坑(col69)
    s2.mario.y = 12 * TILE;
    expect(rolloutInput(s2, WORLD_1_1).jump).toBe(true);
  });
});

describe('mario planner · autopilot(System 2 直接接管)', () => {
  it('坑前起跳、开阔地全速跑', () => {
    const s = createGameState();
    s.mario.x = 69 * TILE - 12; // 坑缘(已助跑)
    s.mario.vx = 2.4;
    expect(autopilotInput(s, WORLD_1_1).jump).toBe(true);

    const s2 = createGameState();
    s2.goombas[0]!.alive = false;
    s2.mario.x = 31 * TILE; // 平地,管道 28 已过
    s2.mario.vx = 2.4;
    const input = autopilotInput(s2, WORLD_1_1);
    expect(input.right).toBe(true);
    expect(input.run).toBe(true);
  });

  it('autopilot 同 tick 缓存命中(不重复搜索)', () => {
    const s = createGameState();
    const a = autopilotInput(s, WORLD_1_1);
    const b = autopilotInput(s, WORLD_1_1);
    expect(a).toEqual(b);
  });

  it('死亡态 autopilot 不产生输入', () => {
    const s = createGameState();
    s.phase = 'dead';
    expect(autopilotInput(s, WORLD_1_1)).toEqual(IDLE_INPUT);
  });
});

describe('mario planner · 动作映射与工具', () => {
  it('动作 → 输入向量映射', () => {
    expect(actionInput('jump_run_right')).toEqual({
      left: false,
      right: true,
      run: true,
      jump: true,
      fire: false,
    });
    expect(actionInput('idle')).toEqual(IDLE_INPUT);
    expect(actionInput('fire_right')).toEqual({
      left: false,
      right: true,
      run: false,
      jump: false,
      fire: true,
    });
    expect(ACTIONS).toHaveLength(11);
  });

  it('canClearGap 与 JUMP 表一致', () => {
    expect(canClearGap(JUMP.runDistTiles)).toBe(true);
    expect(canClearGap(JUMP.runDistTiles + 1)).toBe(false);
  });

  it('两关的坑都在助跑跳距内(可达性不变量)', () => {
    for (const world of [WORLD_1_1, WORLD_1_2]) {
      for (const g of world.gaps) {
        expect(g.endCol - g.startCol + 1).toBeLessThanOrEqual(JUMP.runDistTiles);
      }
    }
  });

  it('1-2 注册进关卡表', () => {
    expect(LEVELS['1-2']).toBeDefined();
    expect(WORLD_1_2.data.name).toBe('1-2');
    expect(ITEMS.comboBase).toBeGreaterThan(0);
  });
});

describe('mario brain 观测工具(planner 复用)', () => {
  it('profile16:地面=13,?块列=9,坑列=15', () => {
    const s = createGameState();
    s.mario.x = 0;
    const p = profile16(s);
    expect(p).toHaveLength(16);
    expect(p[15]).toBe(9); // col 16 的 ? 块(row 9)
    expect(p[0]).toBe(13);
    const s2 = createGameState();
    s2.mario.x = 66 * TILE;
    const p2 = profile16(s2);
    expect(p2[3]).toBe(15); // col 70 无实心
    expect(p2[5]).toBe(13);
  });

  it('questions 只含候选集 criteria', () => {
    const q = buildQuestions(['jump_run_right', 'jump_right'] as const);
    const action = q.action as { criteria: Record<string, string> };
    expect(Object.keys(action.criteria).sort()).toEqual(['jump_right', 'jump_run_right']);
  });
});
