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
import { COLS, GROUND_ROW, ITEMS, JUMP, TILE, WORLD_1_1, WORLD_1_2 } from './world1-1';

function run(s: GameState, input: Input, ticks: number): void {
  for (let i = 0; i < ticks; i += 1) step(WORLD_1_1, s, input);
}

type Seg = [Partial<Input>, number];
const I = (p: Partial<Input>): Input => ({ ...IDLE_INPUT, ...p });

/** 清空所有敌人:让"道具/砖块"这类机制测试只测自己那一件事。 */
function quietState(world = WORLD_1_1): GameState {
  const s = createGameState(world);
  for (const g of s.goombas) g.alive = false;
  for (const k of s.koopas) k.alive = false;
  for (const p of s.plants) p.dead = true;
  return s;
}

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
    const s = quietState();
    s.mario.x = 21 * TILE + 8 - OXShift(); // 头对准 col21 的 coin ? 块
    run(s, I({ jump: true }), 40);
    expect(s.score).toBe(200);
    expect(s.coinCount).toBe(1);
    const b = s.blocks.get(21 * 32 + 9);
    expect(b?.used).toBe(true);
    expect(solidAtPx(s, 21 * TILE + 8, 9 * TILE + 8)).toBe(true);
  });

  it('multi ?块连续吐币(8 枚上限)', () => {
    const s = quietState();
    s.mario.x = 23 * TILE + 8 - OXShift(); // col23 = multi
    // 必须"跳一下 → 松手 → 再跳":jumpLatch 只在 input.jump=false 时清零
    const hop = (): void => {
      run(s, I({ jump: true }), 22);
      run(s, IDLE_INPUT, 26);
    };
    hop();
    expect(s.coinCount).toBe(1);
    hop();
    expect(s.coinCount).toBeGreaterThanOrEqual(2);
    const b = s.blocks.get(23 * 32 + 9);
    expect(b?.coinsLeft).toBeLessThan(8);
    expect(b?.used).toBe(false); // 未吐完不变 used
  });

  it('蘑菇 ?块:顶出蘑菇 → 追到它 → 变大 power=big', () => {
    const s = quietState();
    s.mario.x = 16 * TILE + 8 - OXShift(); // col16 = mushroom
    run(s, I({ jump: true }), 30);
    expect(s.items.some((i) => i.kind === 'mushroom')).toBe(true);
    // 追蘑菇(向右跑)直至吃掉
    let grew = false;
    for (let i = 0; i < 400 && !grew; i += 1) {
      step(WORLD_1_1, s, I({ right: true, run: true }));
      if (s.mario.power !== 'small') grew = true;
    }
    expect(grew).toBe(true);
    expect(s.mario.power).toBe('big');
    expect(s.score).toBeGreaterThanOrEqual(1000);
  });

  it('big 马里奥 顶碎砖块(tile 变空 +50 分)', () => {
    const s = quietState();
    s.mario.power = 'big';
    s.mario.x = 20 * TILE + 8 - OXShift(); // col20 row9 是砖
    run(s, I({ jump: true }), 40);
    expect(s.tiles[9 * COLS + 20]).toBe(0);
    expect(s.blocks.has(20 * 32 + 9)).toBe(false);
    expect(s.score).toBe(50);
  });

  it('small 马里奥 顶砖只顶不碎(砖仍在)', () => {
    const s = quietState();
    s.mario.x = 20 * TILE + 8 - OXShift();
    run(s, I({ jump: true }), 40);
    expect(s.tiles[9 * COLS + 20]).not.toBe(0);
  });

  it('fire 形态可发射火球(按住不连发),火球弹地砸死 Goomba', () => {
    const s = quietState();
    s.mario.power = 'fire';
    const g = s.goombas[0]!;
    g.active = true;
    g.vx = 0;
    s.mario.x = g.x - 70;
    s.mario.y = g.y;
    step(WORLD_1_1, s, I({ right: true, fire: true }));
    expect(s.fireballs).toHaveLength(1);
    // 按住不放不连发(需要松手再按)
    step(WORLD_1_1, s, I({ right: true, fire: true }));
    expect(s.fireballs).toHaveLength(1);
    // 松手 → 再按 → 第二颗(上限 2)
    step(WORLD_1_1, s, I({ right: true }));
    step(WORLD_1_1, s, I({ right: true, fire: true }));
    expect(s.fireballs).toHaveLength(2);
    let killed = false;
    for (let i = 0; i < 200 && !killed; i += 1) {
      step(WORLD_1_1, s, I({ right: true }));
      if (!g.alive) killed = true;
    }
    expect(killed).toBe(true);
  });

  it('big 受伤降级为 small,small 再受伤才死', () => {
    const s = createGameState();
    s.mario.power = 'big';
    s.mario.invuln = 0;
    const g = s.goombas[0]!;
    g.active = true;
    g.vx = 0;
    s.mario.x = g.x - 20;
    s.mario.y = g.y;
    let damaged = false;
    for (let i = 0; i < 40 && !damaged; i += 1) {
      step(WORLD_1_1, s, I({ right: true }));
      if (String(s.mario.power) === 'small') damaged = true;
    }
    expect(damaged).toBe(true);
    expect(s.phase).toBe('running');
    expect(s.mario.invuln).toBeGreaterThan(0);
  });

  it('受击后无敌窗口内不再二次受伤', () => {
    const s = createGameState();
    s.mario.invuln = 30;
    const g = s.goombas[0]!;
    g.active = true;
    g.vx = 0;
    s.mario.x = g.x - 20;
    s.mario.y = g.y;
    run(s, I({ right: true }), 10);
    expect(s.phase).toBe('running');
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

  it('连踩连击:分越踩越高(第 n 踩 = base × n)', () => {
    const s = createGameState();
    for (let n = 1; n <= 4; n += 1) {
      const g = s.goombas[n - 1]!;
      g.active = true;
      s.mario.x = g.x - 2;
      s.mario.y = g.y - 20;
      s.mario.vy = 2;
      s.score = 0;
      s.combo = n - 1; // 已经连了 n-1 脚,这一脚应加成 n
      for (let i = 0; i < 10; i += 1) {
        step(WORLD_1_1, s, IDLE_INPUT);
        if (!g.alive) break;
      }
      expect(g.alive).toBe(false);
      expect(s.score).toBe(ITEMS.comboBase * n);
      // 把马里奥挪回地面,下一轮重来
      s.mario.y = 12 * TILE;
      s.mario.vy = 0;
    }
    expect(s.maxCombo).toBeGreaterThanOrEqual(4);
  });

  it('踩 Koopa → 变壳;侧踢 → 滑行', () => {
    const s = createGameState();
    const k = s.koopas[0]!;
    k.active = true;
    s.mario.x = k.x - 2;
    s.mario.y = k.y - 20;
    s.mario.vy = 2;
    for (let i = 0; i < 12 && k.mode === 'walk'; i += 1) step(WORLD_1_1, s, IDLE_INPUT);
    expect(k.mode).toBe('shell');

    // 侧踢
    s.mario.y = k.y;
    s.mario.x = k.x - 24;
    s.mario.invuln = 0;
    let kicked = false;
    for (let i = 0; i < 40 && !kicked; i += 1) {
      step(WORLD_1_1, s, I({ right: true }));
      if (k.mode === 'slide') kicked = true;
    }
    expect(k.mode).toBe('slide');
  });

  it('滑行龟壳撞死 Goomba(链式清场)', () => {
    const s = createGameState();
    const k = s.koopas[0]!;
    const g = s.goombas[0]!;
    // 摆位:壳在左、Goomba 在右 60px,马里奥 远离且无敌
    k.active = true;
    k.mode = 'slide';
    k.vx = 3.2;
    k.x = 30 * TILE;
    k.y = 12 * TILE;
    g.active = true;
    g.alive = true;
    g.vx = 0;
    g.x = k.x + 60;
    g.y = k.y;
    s.mario.x = 20 * TILE;
    s.mario.y = 12 * TILE;
    s.mario.invuln = 9999;
    for (let i = 0; i < 90 && g.alive; i += 1) step(WORLD_1_1, s, IDLE_INPUT);
    expect(g.alive).toBe(false);
  });

  it('龟壳倒计时归零 → 复活为行走态', () => {
    const s = createGameState();
    const k = s.koopas[0]!;
    k.active = true;
    k.mode = 'shell';
    k.revive = 3;
    for (let i = 0; i < 8; i += 1) step(WORLD_1_1, s, IDLE_INPUT);
    expect(k.mode).toBe('walk');
  });

  it('食人花:伸出时碰到马里奥 → 受伤;缩回时无敌', () => {
    const s = createGameState();
    const p = s.plants[0]!;
    expect(p).toBeDefined();
    // 站到管顶上(管高 4 → topRow 9),与完全伸出的食人花同排
    s.mario.x = p.col * TILE + 4;
    s.mario.y = p.topRow * TILE - TILE;
    s.mario.invuln = 0;
    p.stage = 'out';
    p.out = 1;
    p.timer = 999;
    let hurt = false;
    for (let i = 0; i < 6 && !hurt; i += 1) {
      step(WORLD_1_1, s, IDLE_INPUT);
      hurt = s.phase === 'dead' || s.mario.invuln > 0;
    }
    expect(hurt).toBe(true);
  });

  it('食人花 马里奥贴近时不伸出(经典规避)', () => {
    const s = createGameState();
    const p = s.plants[0]!;
    s.mario.x = p.col * TILE + TILE - 8; // 站在管子上
    p.stage = 'retracted';
    p.out = 0;
    p.timer = 1;
    for (let i = 0; i < 10; i += 1) step(WORLD_1_1, s, IDLE_INPUT);
    expect(p.out).toBe(0);
    expect(p.stage).toBe('retracted');
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

  it('autopilot 通关 1-2(System 2 搜索对全新布局同样有效)', () => {
    return import('./planner').then(({ autopilotInput }) => {
      const s = createGameState(WORLD_1_2);
      let won = false;
      const maxTicks = 60 * 160;
      for (let i = 0; i < maxTicks && !won; i += 1) {
        const input = autopilotInput(s, WORLD_1_2);
        const events = step(WORLD_1_2, s, input);
        if (events.some((e) => e.type === 'win')) won = true;
      }
      expect(won).toBe(true);
      expect(s.score).toBeGreaterThan(0);
    });
  });

  it('通关序列:旗杆下滑 → 走向城堡 → 消失 + 时间奖励结算', () => {
    const s = createGameState();
    s.mario.x = 197 * TILE;
    s.mario.y = 6 * TILE;
    let win: { type: 'win'; bonus: number } | undefined;
    for (let i = 0; i < 40 && !win; i += 1) {
      win = step(WORLD_1_1, s, I({ right: true, run: true })).find(
        (e): e is { type: 'win'; bonus: number } => e.type === 'win',
      );
    }
    expect(win).toBeDefined();
    expect(s.phase).toBe('won');
    expect(s.win.stage).toBe('slide');
    // 跑完整个序列
    for (let i = 0; i < 400; i += 1) step(WORLD_1_1, s, IDLE_INPUT);
    expect(s.win.stage).toBe('enter');
    expect(s.mario.hidden).toBe(true);
    expect(s.timeBonus).toBe(s.timeUnits * ITEMS.timeBonusPerUnit);
  });
});

/** 头部中心对准目标瓦片中心的 x 修正(测试辅助)。 */
function OXShift(): number {
  return 2 + 6; // OX + BW/2 → 盒中心
}
