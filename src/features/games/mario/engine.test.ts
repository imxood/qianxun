import { describe, expect, it } from 'vitest';

import {
  IDLE_INPUT,
  createGameState,
  solidAtPx,
  stateHash,
  step,
  type GameState,
  type Input,
} from './engine';
import { COLS, GROUND_ROW, JUMP, TILE, WORLD_1_1 } from './world1-1';

function run(s: GameState, input: Input, ticks: number): void {
  for (let i = 0; i < ticks; i += 1) step(WORLD_1_1, s, input);
}

type Seg = [Partial<Input>, number];
const I = (p: Partial<Input>): Input => ({ ...IDLE_INPUT, ...p });

describe('mario world 不变量', () => {
  it('所有坑宽 ≤ 助跑跳距(规划器可达性前提)', () => {
    for (const g of WORLD_1_1.gaps) {
      const w = g.endCol - g.startCol + 1;
      expect(w).toBeLessThanOrEqual(JUMP.runDistTiles);
    }
    expect(WORLD_1_1.gaps).toHaveLength(3);
  });

  it('出生点、旗杆座、每个 Goomba 脚下都是实心瓦', () => {
    const d = WORLD_1_1.data;
    expect(WORLD_1_1.tiles[GROUND_ROW * COLS + d.spawn.col]).not.toBe(0);
    expect(WORLD_1_1.tiles[GROUND_ROW * COLS + d.flagCol]).not.toBe(0);
    for (const g of d.goombas) {
      expect(WORLD_1_1.tiles[(g.row + 1) * COLS + g.col]).not.toBe(0);
    }
  });

  it('跳跃可达表来自物理常量:跳高 ≥ 4 瓦(能过 4 高管道)', () => {
    expect(JUMP.heightTiles).toBeGreaterThanOrEqual(4);
    expect(JUMP.runDistTiles).toBeGreaterThanOrEqual(5);
  });
});

describe('mario engine', () => {
  it('确定性:同一输入脚本两次运行,状态指纹逐 tick 一致', () => {
    const play = (): string[] => {
      const s = createGameState();
      const hashes: string[] = [];
      const segs: Seg[] = [
        [{ right: true, run: true }, 90],
        [{ right: true, run: true, jump: true }, 40],
        [{ right: true }, 60],
        [{}, 40],
        [{ left: true }, 30],
      ];
      for (const [p, n] of segs) {
        const input = I(p);
        for (let i = 0; i < n; i += 1) {
          step(WORLD_1_1, s, input);
          hashes.push(stateHash(s));
        }
      }
      return hashes;
    };
    const a = play();
    const b = play();
    expect(a).toEqual(b);
    expect(a.length).toBe(260);
  });

  it('视口无关:不同 viewW 下同一输入,小怪激活/状态完全一致', () => {
    const play = (viewW: number): { hashes: string[]; goombaX: number[] } => {
      const s = createGameState();
      const hashes: string[] = [];
      const input = I({ right: true, run: true });
      for (let i = 0; i < 240; i += 1) {
        step(WORLD_1_1, s, input, viewW);
        hashes.push(stateHash(s));
      }
      return { hashes, goombaX: s.goombas.map((g) => Math.round(g.x)) };
    };
    const a = play(256);
    const b = play(640); // UI 大舞台
    expect(a.hashes).toEqual(b.hashes);
    expect(a.goombaX).toEqual(b.goombaX);
  });

  it('满跳高度 ≈ 4 瓦;松手截断 = 短跳', () => {
    const spawnY = (GROUND_ROW - 1) * TILE;
    const full = createGameState();
    let minY = Infinity;
    for (let i = 0; i < 60; i += 1) {
      step(WORLD_1_1, full, I({ jump: true }));
      minY = Math.min(minY, full.mario.y);
    }
    const heightFull = spawnY - minY;
    expect(heightFull).toBeGreaterThanOrEqual(4 * TILE - 6);
    expect(heightFull).toBeLessThanOrEqual(4 * TILE + 8);

    const short = createGameState();
    let minYShort = Infinity;
    for (let i = 0; i < 6; i += 1) step(WORLD_1_1, short, I({ jump: true }));
    for (let i = 0; i < 54; i += 1) {
      step(WORLD_1_1, short, IDLE_INPUT);
      minYShort = Math.min(minYShort, short.mario.y);
    }
    const heightShort = spawnY - minYShort;
    expect(heightShort).toBeLessThan(heightFull - TILE);
  });

  it('助跑跳距 ≥ 5 瓦(平地起跳到落地的水平位移)', () => {
    const s = createGameState();
    for (let c = 10; c < 40; c += 1) s.tiles[9 * COLS + c] = 0; // 清掉头顶砖/?,避免顶头截断
    s.mario.x = 10 * TILE; // 平地
    run(s, I({ right: true, run: true }), 60); // 先助跑满速
    const startX = s.mario.x;
    run(s, I({ right: true, run: true, jump: true }), 46);
    const dist = s.mario.x - startX;
    expect(dist).toBeGreaterThanOrEqual(5 * TILE);
    expect(dist).toBeLessThanOrEqual(48 * TILE);
  });

  it('走进管道被墙挡住', () => {
    const s = createGameState();
    s.mario.x = 26 * TILE; // 管道在 col 28
    run(s, I({ right: true }), 120);
    expect(s.mario.x).toBeLessThan(28 * TILE);
    expect(s.mario.x).toBeGreaterThan(26 * TILE);
  });

  it('掉坑 → 死亡事件 pit,并计入 deaths', () => {
    const s = createGameState();
    s.mario.x = 69 * TILE + 8; // gap1 上方
    s.mario.y = 8 * TILE;
    s.camX = 1024;
    let phaseAtDeath = '';
    for (let i = 0; i < 120 && !phaseAtDeath; i += 1) {
      const ev = step(WORLD_1_1, s, IDLE_INPUT);
      if (ev.some((e) => e.type === 'death')) phaseAtDeath = s.phase;
    }
    expect(phaseAtDeath).toBe('dead');
    expect(s.deaths[0]?.cause).toBe('pit');
  });

  it('?块顶出金币:分数 +200、瓦片变 used', () => {
    const s = createGameState();
    s.mario.x = 16 * TILE + 8 - OXShift(); // 头对准 col16 的 ? 块
    run(s, I({ jump: true }), 40);
    const events: string[] = [];
    void events;
    expect(s.score).toBe(200);
    expect(s.coinCount).toBe(1);
    const b = s.blocks.get(16 * 32 + 9);
    expect(b?.used).toBe(true);
    expect(solidAtPx(s, 16 * TILE + 8, 9 * TILE + 8)).toBe(true);
  });

  it('踩踏 Goomba:敌人消灭、马里奥反弹', () => {
    const s = createGameState();
    const g = s.goombas[0]!;
    g.active = true;
    s.mario.x = g.x - 2;
    s.mario.y = g.y - 20; // 从头顶上方落下
    s.mario.vy = 2;
    const types: string[] = [];
    for (let i = 0; i < 10 && !types.includes('stomp'); i += 1) {
      types.push(...step(WORLD_1_1, s, IDLE_INPUT).map((e) => e.type));
    }
    expect(types).toContain('stomp');
    expect(g.alive).toBe(false);
    expect(s.mario.vy).toBeLessThan(0);
  });

  it('侧面碰 Goomba → 死亡 goomba', () => {
    const s = createGameState();
    const g = s.goombas[0]!;
    g.active = true;
    g.vx = 0; // 站桩
    s.mario.x = g.x - 20;
    s.mario.y = g.y; // 同一高度
    for (let i = 0; i < 60; i += 1) {
      const ev = step(WORLD_1_1, s, I({ right: true }));
      if (ev.some((e) => e.type === 'death')) break;
    }
    expect(s.phase).toBe('dead');
    expect(s.deaths[0]?.cause).toBe('goomba');
  });

  it('到达旗杆 → won + 高度加分', () => {
    const s = createGameState();
    s.mario.x = 197 * TILE;
    s.mario.y = 6 * TILE; // 高处触旗 → 高加分
    let win: { type: 'win'; bonus: number } | undefined;
    for (let i = 0; i < 40 && !win; i += 1) {
      const ev = step(WORLD_1_1, s, I({ right: true, run: true }));
      win = ev.find((e): e is { type: 'win'; bonus: number } => e.type === 'win');
    }
    expect(win).toBeDefined();
    expect(s.phase).toBe('won');
    expect(win?.bonus).toBeGreaterThanOrEqual(400);
  });

  it('时间耗尽 → 死亡 time;死亡 90 tick 后重生,attempts +1', () => {
    const s = createGameState();
    s.timeUnits = 1;
    run(s, IDLE_INPUT, 30);
    expect(s.phase).toBe('dead');
    expect(s.deaths[0]?.cause).toBe('time');
    run(s, IDLE_INPUT, 95);
    expect(s.phase).toBe('running');
    expect(s.attempts).toBe(2);
    expect(s.mario.x).toBe(WORLD_1_1.data.spawn.col * TILE);
  });

  it('autopilot 全程通关(确定性仿真,≤2 分钟游戏时间)', () => {
    // 引入 planner(避免测试文件间循环依赖,这里动态导入)
    return import('./planner').then(({ autopilotInput }) => {
      const s = createGameState();
      let won = false;
      const maxTicks = 60 * 130;
      for (let i = 0; i < maxTicks && !won; i += 1) {
        const input = autopilotInput(s, WORLD_1_1);
        const events = step(WORLD_1_1, s, input);
        if (events.some((e) => e.type === 'win')) won = true;
      }
      expect(won).toBe(true);
    });
  });
});

/** 头部中心对准目标瓦片中心的 x 修正(测试辅助)。 */
function OXShift(): number {
  return 2 + 6; // OX + BW/2 → 盒中心
}
