/**
 * 马里奥引擎(docs/11 §3):确定性、固定步长(60Hz)、纯逻辑、零依赖、可单测。
 *
 * - `step(state, input)` 就地推进一帧并返回事件流;同输入序列必得同状态
 *   (vitest 以状态哈希锁定)——确定性 = 可单测 + 可回放 + 可批量生成蒸馏数据。
 * - 状态是**普通可变对象**,不进 Svelte 响应式(60fps 数据进 runes 会拖垮
 *   调度器,docs/11 §2 差异①);HUD 由 UI 层 10Hz 快照。
 * - 瓦片网格在 createGameState 时拷贝进 state(?块顶过变 used 会写瓦片),
 *   模块级 WORLD_1_1 保持只读,测试/重开局互不污染。
 * - 无随机源、无时钟读取:v1 全关确定性。
 */

import {
  COLS,
  GROUND_ROW,
  PHYSICS,
  ROWS,
  TILE,
  T_EMPTY,
  T_QBLOCK,
  T_USED,
  WORLD_1_1,
  isSolid,
  tileAt,
  type World,
} from './world1-1';

export type Input = { left: boolean; right: boolean; run: boolean; jump: boolean };
export const IDLE_INPUT: Input = { left: false, right: false, run: false, jump: false };

export type Mario = {
  x: number;
  y: number;
  vx: number;
  vy: number;
  onGround: boolean;
  face: 1 | -1;
  jumpLatch: boolean;
};

export type Goomba = {
  id: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  active: boolean;
  alive: boolean;
  squashT: number;
};

export type CoinEntity = { id: number; col: number; row: number; taken: boolean };
export type BlockEntity = {
  col: number;
  row: number;
  kind: 'q' | 'brick';
  used: boolean;
  bumpT: number;
};

export type DeathCause = 'pit' | 'goomba' | 'time';
export type DeathRecord = { x: number; cause: DeathCause };

export type GameEvent =
  | { type: 'jump' }
  | { type: 'coin'; source: 'float' | 'block' }
  | { type: 'stomp' }
  | { type: 'bump'; kind: 'brick' | 'q' }
  | { type: 'death'; cause: DeathCause }
  | { type: 'respawn' }
  | { type: 'win'; bonus: number };

export type Phase = 'running' | 'dead' | 'won';

export type GameState = {
  tick: number;
  phase: Phase;
  mario: Mario;
  goombas: Goomba[];
  coins: CoinEntity[];
  blocks: Map<number, BlockEntity>; // key = col*32+row
  tiles: Uint8Array; // 世界瓦片的本局副本(?→used 写这里)
  score: number;
  coinCount: number;
  timeUnits: number;
  timeCarry: number;
  attempts: number;
  camX: number;
  maxX: number;
  deaths: DeathRecord[];
  deadTimer: number;
};

// ---- 碰撞盒:精灵 16×16,实体盒 12×14(左右各缩 2,顶部缩 2,脚底齐平) ----
const OX = 2;
const BW = 12;
const OY_TOP = 2;
const FOOT = 16; // y + FOOT = 脚底

/** 每单位时间(1 game-unit ≈ 0.4s)对应的 tick 数。 */
const TIME_TICKS = 24;
const DEAD_TICKS = 90;

export function createGameState(world: World = WORLD_1_1): GameState {
  const d = world.data;
  return {
    tick: 0,
    phase: 'running',
    mario: {
      x: d.spawn.col * TILE,
      y: GROUND_ROW * TILE - TILE,
      vx: 0,
      vy: 0,
      onGround: true,
      face: 1,
      jumpLatch: false,
    },
    goombas: d.goombas.map((g, id) => ({
      id,
      x: g.col * TILE,
      y: (g.row + 1) * TILE - TILE,
      vx: -PHYSICS.enemySpeed,
      vy: 0,
      active: false,
      alive: true,
      squashT: 0,
    })),
    coins: d.coins.map((c, id) => ({ id, col: c.col, row: c.row, taken: false })),
    blocks: new Map(
      [
        ...d.qblocks.map((b) => ({ ...b, kind: 'q' as const })),
        ...d.bricks.map((b) => ({ ...b, kind: 'brick' as const })),
      ].map((b) => [b.col * 32 + b.row, { ...b, used: false, bumpT: 0 }]),
    ),
    tiles: world.tiles.slice(),
    score: 0,
    coinCount: 0,
    timeUnits: d.timeLimit,
    timeCarry: 0,
    attempts: 1,
    camX: 0,
    maxX: d.spawn.col * TILE,
    deaths: [],
    deadTimer: 0,
  };
}

function overlap(
  ax: number,
  ay: number,
  aw: number,
  ah: number,
  bx: number,
  by: number,
  bw: number,
  bh: number,
): boolean {
  return ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by;
}

function marioBox(x: number, y: number): { l: number; r: number; t: number; b: number } {
  return { l: x + OX, r: x + OX + BW, t: y + OY_TOP, b: y + FOOT };
}

export function step(world: World, s: GameState, input: Input, viewW = 256): GameEvent[] {
  const events: GameEvent[] = [];
  s.tick += 1;

  if (s.phase === 'dead') {
    s.deadTimer -= 1;
    if (s.deadTimer <= 0) respawn(world, s, events);
    return events;
  }
  if (s.phase === 'won') return events;

  // ---- 时间 ----
  s.timeCarry += 1;
  if (s.timeCarry >= TIME_TICKS) {
    s.timeCarry = 0;
    s.timeUnits -= 1;
    if (s.timeUnits <= 0) {
      die(s, events, 'time');
      return events;
    }
  }

  const m = s.mario;

  // ---- 水平:加速 / 摩擦 / 走跑上限 ----
  const dir = input.right && !input.left ? 1 : input.left && !input.right ? -1 : 0;
  if (dir !== 0) {
    m.vx += dir * (input.run ? PHYSICS.runAcc : PHYSICS.walkAcc);
    m.face = dir;
  } else {
    if (m.vx > 0) m.vx = Math.max(0, m.vx - PHYSICS.friction);
    else if (m.vx < 0) m.vx = Math.min(0, m.vx + PHYSICS.friction);
  }
  const maxSpeed = input.run ? PHYSICS.maxRun : PHYSICS.maxWalk;
  if (Math.abs(m.vx) > maxSpeed)
    m.vx = Math.sign(m.vx) * Math.max(maxSpeed, Math.abs(m.vx) - PHYSICS.friction * 2);

  // ---- 跳跃:边缘触发 + 松手截断(变高跳) ----
  if (input.jump && m.onGround && !m.jumpLatch) {
    m.vy = PHYSICS.jumpV;
    m.onGround = false;
    m.jumpLatch = true;
    events.push({ type: 'jump' });
  }
  if (!input.jump) m.jumpLatch = false;

  const gravity =
    m.vy < 0 ? (input.jump ? PHYSICS.gravityUp : PHYSICS.gravityCut) : PHYSICS.gravityFall;
  m.vy = Math.min(m.vy + gravity, PHYSICS.maxFall);

  // ---- 水平位移 + 瓦片碰撞 ----
  let newX = m.x + m.vx;
  {
    const t = Math.floor((m.y + OY_TOP) / TILE);
    const b = Math.floor((m.y + FOOT - 1) / TILE);
    if (m.vx > 0) {
      const col = Math.floor((newX + OX + BW - 1) / TILE);
      for (let r = t; r <= b; r += 1) {
        if (isSolid(s.tiles[r * COLS + col] ?? 0)) {
          newX = col * TILE - OX - BW;
          m.vx = 0;
          break;
        }
      }
    } else if (m.vx < 0) {
      const col = Math.floor((newX + OX) / TILE);
      for (let r = t; r <= b; r += 1) {
        if (isSolid(s.tiles[r * COLS + col] ?? 0)) {
          newX = (col + 1) * TILE - OX;
          m.vx = 0;
          break;
        }
      }
    }
  }

  // ---- 垂直位移 + 瓦片碰撞 ----
  let newY = m.y + m.vy;
  let grounded = false;
  {
    const c0 = Math.floor((newX + OX) / TILE);
    const c1 = Math.floor((newX + OX + BW - 1) / TILE);
    if (m.vy > 0) {
      const row = Math.floor((newY + FOOT) / TILE);
      for (let c = c0; c <= c1; c += 1) {
        if (isSolid(s.tiles[row * COLS + c] ?? 0)) {
          newY = row * TILE - FOOT;
          m.vy = 0;
          grounded = true;
          break;
        }
      }
    } else if (m.vy < 0) {
      const row = Math.floor((newY + OY_TOP) / TILE);
      let hitCol = -1;
      let hitOverlap = 0;
      for (let c = c0; c <= c1; c += 1) {
        if (isSolid(s.tiles[row * COLS + c] ?? 0)) {
          const oL = Math.max(newX + OX, c * TILE);
          const oR = Math.min(newX + OX + BW, (c + 1) * TILE);
          const o = oR - oL;
          if (o > hitOverlap) {
            hitOverlap = o;
            hitCol = c;
          }
        }
      }
      if (hitCol >= 0) {
        newY = (row + 1) * TILE - OY_TOP;
        m.vy = 0;
        headHit(world, s, events, hitCol, row);
      }
    }
  }
  m.onGround = grounded;
  m.x = newX;
  m.y = newY;
  if (m.x > s.maxX) s.maxX = m.x;

  // ---- 掉坑 ----
  if (m.y > ROWS * TILE + TILE) {
    die(s, events, 'pit');
    return events;
  }

  // ---- 相机:只进不退,马里奥锚在视口 38% 处 ----
  s.camX = Math.min(Math.max(s.camX, m.x - viewW * 0.38), COLS * TILE - viewW);
  if (s.camX < 0) s.camX = 0;

  // ---- 金币 ----
  const mb = marioBox(m.x, m.y);
  for (const c of s.coins) {
    if (c.taken) continue;
    if (overlap(mb.l, mb.t, BW, FOOT - OY_TOP, c.col * TILE + 3, c.row * TILE + 3, 10, 10)) {
      c.taken = true;
      s.coinCount += 1;
      s.score += 100;
      events.push({ type: 'coin', source: 'float' });
    }
  }

  // ---- Goomba ----
  for (const g of s.goombas) {
    // 激活锚定马里奥(288px ≈ 18 瓦),与视宽/相机解耦——引擎必须视口无关:
    // camX 随视宽钳制而不同、按相机激活会让同一输入在不同窗口尺寸下走出
    // 不同的局(实测:自驾在 UI 里反复死于 goomba@161,测试里却全通)。
    if (!g.active && g.x < s.mario.x + 288) g.active = true;
    if (g.squashT > 0) g.squashT -= 1;
    if (!g.alive || !g.active) continue;

    g.vy = Math.min(g.vy + PHYSICS.gravityFall, PHYSICS.maxFall);
    let nx = g.x + g.vx;
    {
      const t = Math.floor((g.y + 2) / TILE);
      const b = Math.floor((g.y + 15) / TILE);
      const edge = g.vx < 0 ? nx + 1 : nx + 14;
      const col = Math.floor(edge / TILE);
      for (let r = t; r <= b; r += 1) {
        if (isSolid(s.tiles[r * COLS + col] ?? 0)) {
          g.vx = -g.vx;
          nx = g.x;
          break;
        }
      }
    }
    let ny = g.y + g.vy;
    if (g.vy > 0) {
      const row = Math.floor((ny + 16) / TILE);
      for (let c = Math.floor((nx + 2) / TILE); c <= Math.floor((nx + 13) / TILE); c += 1) {
        if (isSolid(s.tiles[row * COLS + c] ?? 0)) {
          ny = row * TILE - 16;
          g.vy = 0;
          break;
        }
      }
    }
    g.x = nx;
    g.y = ny;
    if (g.y > ROWS * TILE + 32) {
      g.alive = false; // 走进坑里消失(经典行为)
      continue;
    }

    // 与马里奥接触:下落中且脚在敌上半部 → 踩踏;否则死亡
    if (overlap(mb.l, mb.t, BW, FOOT - OY_TOP, g.x + 1, g.y + 2, 14, 14)) {
      if (m.vy > 0 && mb.b - g.y < 10) {
        g.alive = false;
        g.squashT = 30;
        m.vy = PHYSICS.stompV;
        s.score += 100;
        events.push({ type: 'stomp' });
      } else {
        die(s, events, 'goomba');
        return events;
      }
    }
  }

  // ---- 旗杆 ----
  const flagX = world.data.flagCol * TILE + 6;
  if (m.x + OX + BW >= flagX) {
    const bonusRows = Math.min(8, Math.max(1, GROUND_ROW - Math.floor((m.y + FOOT) / TILE)));
    const bonus = bonusRows * 100;
    s.score += bonus;
    s.phase = 'won';
    events.push({ type: 'win', bonus });
  }

  return events;
}

function headHit(world: World, s: GameState, events: GameEvent[], col: number, row: number): void {
  const key = col * 32 + row;
  const tile = s.tiles[row * COLS + col] ?? 0;
  if (tile === T_QBLOCK) {
    s.tiles[row * COLS + col] = T_USED;
    const b = s.blocks.get(key);
    if (b) {
      b.used = true;
      b.bumpT = 8;
    }
    s.score += 200;
    s.coinCount += 1;
    events.push({ type: 'coin', source: 'block' });
    events.push({ type: 'bump', kind: 'q' });
    void world;
  } else if (tile !== T_EMPTY) {
    const b = s.blocks.get(key);
    if (b && !b.used) b.bumpT = 8;
    events.push({ type: 'bump', kind: 'brick' });
  }
}

function die(s: GameState, events: GameEvent[], cause: DeathCause): void {
  s.phase = 'dead';
  s.deadTimer = DEAD_TICKS;
  s.deaths.push({ x: s.mario.x, cause });
  events.push({ type: 'death', cause });
}

function respawn(world: World, s: GameState, events: GameEvent[]): void {
  const d = world.data;
  s.attempts += 1;
  s.mario = {
    x: d.spawn.col * TILE,
    y: GROUND_ROW * TILE - TILE,
    vx: 0,
    vy: 0,
    onGround: true,
    face: 1,
    jumpLatch: false,
  };
  s.goombas = d.goombas.map((g, id) => ({
    id,
    x: g.col * TILE,
    y: (g.row + 1) * TILE - TILE,
    vx: -PHYSICS.enemySpeed,
    vy: 0,
    active: false,
    alive: true,
    squashT: 0,
  }));
  s.timeUnits = d.timeLimit;
  s.timeCarry = 0;
  s.camX = 0;
  s.phase = 'running';
  events.push({ type: 'respawn' });
}

/** 供 UI/测试读取的实心判定(走本局副本)。 */
export function solidAtPx(s: GameState, px: number, py: number): boolean {
  return isSolid(tileAt({ ...WORLD_1_1, tiles: s.tiles }, px, py));
}

/** 状态指纹(测试锁定确定性;遥测里做 stale 守卫的辅助)。 */
export function stateHash(s: GameState): string {
  return JSON.stringify({
    t: s.tick,
    p: s.phase,
    m: [s.mario.x, s.mario.y, s.mario.vx, s.mario.vy],
    s: s.score,
    g: s.goombas.filter((g) => g.alive).map((g) => [g.x, g.y]),
  });
}
