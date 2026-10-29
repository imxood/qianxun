/**
 * 马里奥引擎(docs/11 §3):确定性、固定步长(60Hz)、纯逻辑、零依赖、可单测。
 *
 * v2 扩展(2026-10):变身/受伤、龟壳、食人花、道具、碎砖、火球、旗杆序列。
 * - `step(state, input)` 就地推进一帧并返回事件流;同输入序列必得同状态
 *   (vitest 以状态哈希锁定)——确定性 = 可单测 + 可回放 + 可批量生成蒸馏数据。
 * - 状态是**普通可变对象**,不进 Svelte 响应式(60fps 数据进 runes 会拖垮
 *   调度器,docs/11 §2 差异①);HUD 由 UI 层 10Hz 快照。
 * - 瓦片网格在 createGameState 时拷贝进 state(?块顶过变 used 会写瓦片),
 *   模块级 WORLD_* 保持只读,测试/重开局互不污染。
 * - 无随机源、无时钟读取:全关确定性。
 * - **物理常量对 small/big/fire 三态一致**(不做"变大跳更高"):JUMP 表因此
 *   对三种形态同时有效,规划器可达性推理无需分叉。
 */

import {
  COLS,
  GROUND_ROW,
  ITEMS,
  PHYSICS,
  ROWS,
  TILE,
  T_EMPTY,
  T_QBLOCK,
  T_USED,
  isPipeCol,
  isSolid,
  pipeTopRow,
  tileAt,
  WORLD_1_1,
  type World,
} from './world1-1';

export type Input = { left: boolean; right: boolean; run: boolean; jump: boolean; fire: boolean };
export const IDLE_INPUT: Input = {
  left: false,
  right: false,
  run: false,
  jump: false,
  fire: false,
};

/** 形态:small → big(蘑菇) → fire(花);受击 fire→big→small→死。 */
export type Power = 'small' | 'big' | 'fire';

export type Mario = {
  x: number;
  y: number;
  vx: number;
  vy: number;
  onGround: boolean;
  face: 1 | -1;
  jumpLatch: boolean;
  power: Power;
  /** 受击后无敌闪烁剩余 tick。 */
  invuln: number;
  /** 变身定格剩余 tick(期间不接受输入)。 */
  freeze: number;
  /** 连续滞空 tick(落地清零,用于连击)。 */
  airTicks: number;
  /** 已进入城堡(通关序列末尾,不再绘制/碰撞)。 */
  hidden: boolean;
  /** 发火键 latch(与 jump 同理:按住不放不连发)。 */
  fireLatch: boolean;
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

export type KoopaMode = 'walk' | 'shell' | 'slide';
export type Koopa = {
  id: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  active: boolean;
  alive: boolean;
  mode: KoopaMode;
  /** 静止壳复活倒计时(仅 mode==='shell')。 */
  revive: number;
  /** 被踢后的防误触冷却 tick。 */
  kickLock: number;
};

export type ItemKind = 'mushroom' | 'flower' | 'coin';
export type Item = {
  id: number;
  kind: ItemKind;
  x: number;
  y: number;
  vx: number;
  vy: number;
  /** 从 ?块里升出的剩余 tick(>0 时不可交互、纯视觉上移)。 */
  emerge: number;
  gone: boolean;
};

export type PlantStage = 'retracted' | 'rising' | 'out' | 'falling';
export type Plant = {
  id: number;
  col: number;
  topRow: number;
  /** 伸出比例 0..1。 */
  out: number;
  stage: PlantStage;
  timer: number;
  dead: boolean;
};

export type Fireball = {
  id: number;
  x: number;
  y: number;
  vx: number;
  vy: number;
  life: number;
};

export type CoinEntity = { id: number; col: number; row: number; taken: boolean };
export type BlockEntity = {
  col: number;
  row: number;
  kind: 'q' | 'brick';
  /** 产出物(仅 ?块)。 */
  item: 'coin' | 'multi' | 'mushroom' | 'flower' | null;
  used: boolean;
  /** 连续吐币剩余次数(仅 multi)。 */
  coinsLeft: number;
  bumpT: number;
};

export type DeathCause = 'pit' | 'goomba' | 'koopa' | 'plant' | 'shell' | 'time';
export type DeathRecord = { x: number; cause: DeathCause };

export type WinStage = 'none' | 'slide' | 'walk' | 'enter';
export type WinSeq = { stage: WinStage; t: number; slideFromY: number };

export type GameEvent =
  | { type: 'jump' }
  | { type: 'coin'; source: 'float' | 'block' | 'item' }
  | { type: 'stomp'; what: 'goomba' | 'koopa' | 'shell' }
  | { type: 'shellkick' }
  | { type: 'brickbreak' }
  | { type: 'bump'; kind: 'brick' | 'q' }
  | { type: 'powerup'; to: Power }
  | { type: 'damage'; to: Power }
  | { type: 'shoot' }
  | { type: 'death'; cause: DeathCause }
  | { type: 'respawn' }
  | { type: 'flag' }
  | { type: 'win'; bonus: number };

export type Phase = 'running' | 'dead' | 'won';

export type GameState = {
  tick: number;
  phase: Phase;
  mario: Mario;
  goombas: Goomba[];
  koopas: Koopa[];
  plants: Plant[];
  items: Item[];
  fireballs: Fireball[];
  coins: CoinEntity[];
  blocks: Map<number, BlockEntity>; // key = col*32+row
  tiles: Uint8Array; // 世界瓦片的本局副本(?→used / 碎砖→空 写这里)
  score: number;
  coinCount: number;
  timeUnits: number;
  timeCarry: number;
  attempts: number;
  camX: number;
  maxX: number;
  deaths: DeathRecord[];
  deadTimer: number;
  /** 当前连续连击数(滞空连踩)。 */
  combo: number;
  maxCombo: number;
  /** 通关序列(旗杆下滑 → 走向城堡 → 消失)。 */
  win: WinSeq;
  /** 结算后的时间奖励分。 */
  timeBonus: number;
  /** 累计用时 tick(结算展示用)。 */
  elapsed: number;
};

// ---- 碰撞盒:精灵 16 宽,实体盒 12 宽(左右各缩 2);small 高 14、big/fire 高 26 ----
const OX = 2;
const BW = 12;
const FOOT = 16; // y + FOOT = 脚底
const H_SMALL = 14;
const H_BIG = 26;
/** 一次最多同时存在 2 颗火球(SMB 规则)。 */
const MAX_FIREBALLS = 2;
/** 火球存活 tick(约 2.5s)。 */
const FIREBALL_LIFE = 150;
/** 火球初速与重力(略轻于马里奥,跳得矮但飞得远)。 */
const FIREBALL_VX = 3.0;
const FIREBALL_VY = 1.2;

/** 每单位时间(1 game-unit ≈ 0.4s)对应的 tick 数。 */
const TIME_TICKS = 24;
const DEAD_TICKS = 90;
/** 通关序列各阶段时长(tick)。 */
const SLIDE_TICKS = 96;
const CASTLE_TICKS = 120;

/** 形态 → 碰撞盒高度。 */
export function bodyHeight(power: Power): number {
  return power === 'small' ? H_SMALL : H_BIG;
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

function marioBox(
  x: number,
  y: number,
  power: Power,
): { l: number; r: number; t: number; b: number } {
  const h = bodyHeight(power);
  return { l: x + OX, r: x + OX + BW, t: y + FOOT - h, b: y + FOOT };
}

function newMario(col: number): Mario {
  return {
    x: col * TILE,
    y: GROUND_ROW * TILE - TILE,
    vx: 0,
    vy: 0,
    onGround: true,
    face: 1,
    jumpLatch: false,
    power: 'small',
    invuln: 0,
    freeze: 0,
    airTicks: 0,
    hidden: false,
    fireLatch: false,
  };
}

function spawnGoombas(world: World): Goomba[] {
  return world.data.goombas.map((g, id) => ({
    id,
    x: g.col * TILE,
    y: (g.row + 1) * TILE - TILE,
    vx: -PHYSICS.enemySpeed,
    vy: 0,
    active: false,
    alive: true,
    squashT: 0,
  }));
}

function spawnKoopas(world: World): Koopa[] {
  return world.data.koopas.map((k, id) => ({
    id,
    x: k.col * TILE,
    y: (k.row + 1) * TILE - TILE,
    vx: -PHYSICS.enemySpeed * 0.8,
    vy: 0,
    active: false,
    alive: true,
    mode: 'walk',
    revive: 0,
    kickLock: 0,
  }));
}

function spawnPlants(world: World): Plant[] {
  return world.data.plants
    .filter((p) => isPipeCol(world.data, p.col))
    .map((p, id) => {
      const topRow = pipeTopRow(world.data, p.col);
      // 初始相位按列错开,避免全场同步伸缩(确定性:由列号推导)
      const phase = ((p.col * ITEMS.plantHoldTicks) % ITEMS.plantExtendTicks) | 0;
      return {
        id,
        col: p.col,
        topRow,
        out: 0,
        stage: 'retracted' as PlantStage,
        timer: ITEMS.plantHoldTicks - phase,
        dead: false,
      };
    });
}

export function createGameState(world: World = WORLD_1_1): GameState {
  const d = world.data;
  return {
    tick: 0,
    phase: 'running',
    mario: newMario(d.spawn.col),
    goombas: spawnGoombas(world),
    koopas: spawnKoopas(world),
    plants: spawnPlants(world),
    items: [],
    fireballs: [],
    coins: d.coins.map((c, id) => ({ id, col: c.col, row: c.row, taken: false })),
    blocks: new Map(
      [
        ...d.qblocks.map((b) => ({
          col: b.col,
          row: b.row,
          kind: 'q' as const,
          item: b.item,
          used: false,
          coinsLeft: b.item === 'multi' ? 8 : 0,
          bumpT: 0,
        })),
        ...d.bricks.map((b) => ({
          col: b.col,
          row: b.row,
          kind: 'brick' as const,
          item: null,
          used: false,
          coinsLeft: 0,
          bumpT: 0,
        })),
      ].map((b) => [b.col * 32 + b.row, b]),
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
    combo: 0,
    maxCombo: 0,
    win: { stage: 'none', t: 0, slideFromY: 0 },
    timeBonus: 0,
    elapsed: 0,
  };
}

export function step(world: World, s: GameState, input: Input, viewW = 256): GameEvent[] {
  const events: GameEvent[] = [];
  s.tick += 1;

  if (s.phase === 'dead') {
    s.deadTimer -= 1;
    if (s.deadTimer <= 0) respawn(world, s, events);
    return events;
  }
  if (s.phase === 'won') {
    stepWinSequence(world, s, input, events);
    return events;
  }

  s.elapsed += 1;
  const m = s.mario;

  // ---- 变身定格:不接受输入,只推进定格计时 ----
  if (m.freeze > 0) {
    m.freeze -= 1;
    stepPlants(world, s, events);
    stepItems(world, s, events);
    stepEnemies(world, s, events);
    return events;
  }

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

  if (m.invuln > 0) m.invuln -= 1;

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

  // ---- 发射火球(fire 形态,边缘触发,最多 2 颗) ----
  if (input.fire && m.power === 'fire' && !m.fireLatch && s.fireballs.length < MAX_FIREBALLS) {
    m.fireLatch = true;
    s.fireballs.push({
      id: s.tick,
      x: m.x + OX + (m.face === 1 ? BW : -6),
      y: m.y + FOOT - bodyHeight(m.power) + 8,
      vx: FIREBALL_VX * m.face,
      vy: FIREBALL_VY,
      life: FIREBALL_LIFE,
    });
    events.push({ type: 'shoot' });
  }
  if (!input.fire) m.fireLatch = false;

  const box = marioBox(m.x, m.y, m.power);

  // ---- 水平位移 + 瓦片碰撞 ----
  let newX = m.x + m.vx;
  {
    const t = Math.floor(box.t / TILE);
    const b = Math.floor((box.b - 1) / TILE);
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
      const row = Math.floor((newY + FOOT - bodyHeight(m.power)) / TILE);
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
        newY = (row + 1) * TILE - (FOOT - bodyHeight(m.power));
        m.vy = 0;
        headHit(world, s, events, hitCol, row);
      }
    }
  }
  m.onGround = grounded;
  if (!m.onGround) m.airTicks += 1;
  else if (m.airTicks > 0) {
    m.airTicks = 0;
    s.combo = 0;
  }
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

  const mb = marioBox(m.x, m.y, m.power);
  const mw = BW;
  const mh = bodyHeight(m.power);

  // ---- 金币 ----
  for (const c of s.coins) {
    if (c.taken) continue;
    if (overlap(mb.l, mb.t, mw, mh, c.col * TILE + 3, c.row * TILE + 3, 10, 10)) {
      c.taken = true;
      s.coinCount += 1;
      s.score += 200;
      events.push({ type: 'coin', source: 'float' });
    }
  }

  stepItems(world, s, events);
  stepPlants(world, s, events);
  stepEnemies(world, s, events);
  if (enemyCollisions(world, s, mb, mw, mh, events)) return events;
  stepFireballs(world, s, events);

  // ---- 旗杆 ----
  const flagX = world.data.flagCol * TILE + 6;
  if (m.x + OX + BW >= flagX) startWinSequence(world, s, events);

  return events;
}

// ---------------------------------------------------------------- 道具

function stepItems(world: World, s: GameState, events: GameEvent[]): void {
  const m = s.mario;
  for (const it of s.items) {
    if (it.gone) continue;
    if (it.emerge > 0) {
      it.emerge -= 1;
      it.y -= 0.5; // 从块里匀速升出
      continue;
    }
    if (it.kind === 'coin') {
      // 吐出的金币:抛物线飞出后自毁
      it.vy = Math.min(it.vy + PHYSICS.gravityFall, PHYSICS.maxFall);
      it.x += it.vx;
      it.y += it.vy;
      if (it.y > (GROUND_ROW + 1) * TILE) it.gone = true;
      continue;
    }
    if (it.kind === 'mushroom') {
      it.vy = Math.min(it.vy + PHYSICS.gravityFall, PHYSICS.maxFall);
      const nx = it.x + it.vx;
      // 撞墙折返
      const col = Math.floor((it.vx > 0 ? nx + 14 : nx) / TILE);
      const row = Math.floor((it.y + 12) / TILE);
      if (isSolid(s.tiles[row * COLS + col] ?? 0)) {
        it.vx = -it.vx;
      } else {
        it.x = nx;
      }
      it.y += it.vy;
      // 落地
      const belowRow = Math.floor((it.y + 15) / TILE);
      const bc0 = Math.floor((it.x + 2) / TILE);
      const bc1 = Math.floor((it.x + 13) / TILE);
      for (let c = bc0; c <= bc1; c += 1) {
        if (isSolid(s.tiles[belowRow * COLS + c] ?? 0)) {
          it.y = belowRow * TILE - 16;
          it.vy = 0;
          break;
        }
      }
    }
    // 吃到道具
    const box = marioBox(m.x, m.y, m.power);
    if (overlap(box.l, box.t, BW, bodyHeight(m.power), it.x + 1, it.y + 1, 14, 14)) {
      it.gone = true;
      s.score += 1000;
      if (it.kind === 'mushroom') {
        powerUp(s, m.power === 'small' ? 'big' : m.power, events);
      } else {
        powerUp(s, 'fire', events);
      }
    }
  }
  if (s.items.length > 0) s.items = s.items.filter((i) => !i.gone);
  void world;
}

// ---------------------------------------------------------------- 变身 / 受伤

function powerUp(s: GameState, to: Power, events: GameEvent[]): void {
  const m = s.mario;
  if (m.power === to) return;
  // 碰撞盒以脚底为锚(见 marioBox),因此变形只需改 power,y 保持不变即站得住。
  m.power = to;
  m.freeze = ITEMS.growFreezeTicks;
  m.invuln = Math.max(m.invuln, ITEMS.invulnTicks);
  events.push({ type: 'powerup', to });
}

/** 受击:fire→big→small→死。返回 true 表示已死亡。 */
function damage(s: GameState, events: GameEvent[], cause: DeathCause): boolean {
  const m = s.mario;
  if (m.invuln > 0) return false;
  if (m.power !== 'small') {
    m.power = m.power === 'fire' ? 'big' : 'small';
    m.invuln = ITEMS.invulnTicks;
    m.freeze = ITEMS.growFreezeTicks;
    s.combo = 0;
    events.push({ type: 'damage', to: m.power });
    return false;
  }
  die(s, events, cause);
  return true;
}

// ---------------------------------------------------------------- 食人花

function stepPlants(world: World, s: GameState, events: GameEvent[]): void {
  const m = s.mario;
  for (const p of s.plants) {
    if (p.dead) continue;
    // 马里奥贴近管子时不伸出(经典行为)
    const center = p.col * TILE + TILE;
    const marioNear = Math.abs(m.x + 8 - center) < 2.2 * TILE;

    p.timer -= 1;
    switch (p.stage) {
      case 'retracted':
        if (p.timer <= 0 && !marioNear) {
          p.stage = 'rising';
          p.timer = ITEMS.plantExtendTicks;
        }
        break;
      case 'rising':
        p.out = Math.min(1, p.out + 1 / ITEMS.plantExtendTicks);
        if (p.timer <= 0 || p.out >= 1) {
          p.stage = 'out';
          p.timer = ITEMS.plantHoldTicks;
        }
        break;
      case 'out':
        if (p.timer <= 0 || marioNear) {
          p.stage = 'falling';
          p.timer = ITEMS.plantRetractTicks;
        }
        break;
      case 'falling':
        p.out = Math.max(0, p.out - 1 / ITEMS.plantRetractTicks);
        if (p.timer <= 0 || p.out <= 0) {
          p.stage = 'retracted';
          p.timer = ITEMS.plantHoldTicks;
          p.out = 0;
        }
        break;
    }

    if (p.out <= 0.25) continue;
    const h = 2 * TILE * p.out;
    const top = p.topRow * TILE - h;
    if (
      overlap(
        m.x + OX,
        m.y + FOOT - bodyHeight(m.power),
        BW,
        bodyHeight(m.power),
        p.col * TILE + 2,
        top,
        2 * TILE - 4,
        h,
      )
    ) {
      if (damage(s, events, 'plant')) return;
    }
  }
  void world;
  void events;
}

// ---------------------------------------------------------------- 敌人

/** 敌人公共的重力 + 落地/撞墙处理;返回 true 表示已落地。 */
function moveEnemy(
  s: GameState,
  e: { x: number; y: number; vx: number; vy: number },
  w: number,
  h: number,
): void {
  e.vy = Math.min(e.vy + PHYSICS.gravityFall, PHYSICS.maxFall);
  let nx = e.x + e.vx;
  if (e.vx !== 0) {
    const edge = e.vx < 0 ? nx : nx + w - 1;
    const col = Math.floor(edge / TILE);
    const t = Math.floor((e.y + 2) / TILE);
    const b = Math.floor((e.y + h - 1) / TILE);
    for (let r = t; r <= b; r += 1) {
      if (isSolid(s.tiles[r * COLS + col] ?? 0)) {
        e.vx = -e.vx;
        nx = e.x;
        break;
      }
    }
  }
  e.x = nx;
  e.y += e.vy;
  if (e.vy > 0) {
    const row = Math.floor((e.y + h) / TILE);
    const c0 = Math.floor((e.x + 1) / TILE);
    const c1 = Math.floor((e.x + w - 2) / TILE);
    for (let c = c0; c <= c1; c += 1) {
      if (isSolid(s.tiles[row * COLS + c] ?? 0)) {
        e.y = row * TILE - h;
        e.vy = 0;
        break;
      }
    }
  }
}

/** 敌人自主运动(不含与马里奥的交互;碰撞在 enemyCollisions)。 */
function stepEnemies(world: World, s: GameState, events: GameEvent[]): void {
  const m = s.mario;
  for (const g of s.goombas) {
    if (!g.alive) continue;
    if (!g.active && g.x < m.x + 288) g.active = true;
    if (!g.active) continue;
    moveEnemy(s, g, 16, 16);
    if (g.y > ROWS * TILE + 32) g.alive = false;
  }
  for (const k of s.koopas) {
    if (!k.alive) continue;
    if (!k.active && k.x < m.x + 288) k.active = true;
    if (!k.active) continue;
    if (k.kickLock > 0) k.kickLock -= 1;
    if (k.mode === 'shell') {
      k.revive -= 1;
      if (k.revive <= 0) {
        // 壳倒计时归零 → 复活为行走态(不产生事件:不是一次交互)
        k.mode = 'walk';
        k.vx = -PHYSICS.enemySpeed * 0.8;
      }
    }
    moveEnemy(s, k, 16, k.mode === 'walk' ? 16 : 14);
    if (k.y > ROWS * TILE + 32) k.alive = false;
  }
  void world;
  void events;
}

/** 敌人与马里奥的交互判定;返回 true 表示马里奥死亡(调用方应立即 return)。 */
function enemyCollisions(
  world: World,
  s: GameState,
  mb: { l: number; r: number; t: number; b: number },
  mw: number,
  mh: number,
  events: GameEvent[],
): boolean {
  const m = s.mario;
  // ---- Goomba ----
  for (const g of s.goombas) {
    if (!g.alive || !g.active) continue;
    if (g.squashT > 0) g.squashT -= 1;
    if (!overlap(mb.l, mb.t, mw, mh, g.x + 1, g.y + 2, 14, 14)) continue;
    if (m.vy > 0 && mb.b - g.y < 10) {
      stompCombo(s, events);
      g.alive = false;
      g.squashT = 30;
      m.vy = PHYSICS.stompV;
      s.score += comboPoints(s);
      events.push({ type: 'stomp', what: 'goomba' });
    } else if (damage(s, events, 'goomba')) {
      return true;
    }
  }

  // ---- Koopa / 龟壳 ----
  for (const k of s.koopas) {
    if (!k.alive || !k.active) continue;
    const bodyH = k.mode === 'walk' ? 16 : 14;
    if (!overlap(mb.l, mb.t, mw, mh, k.x + 1, k.y + (16 - bodyH), 14, bodyH)) continue;

    if (k.mode === 'walk') {
      if (m.vy > 0 && mb.b - k.y < 12) {
        stompCombo(s, events);
        k.mode = 'shell';
        k.vx = 0;
        k.vy = 0;
        k.revive = ITEMS.shellReviveTicks;
        m.vy = PHYSICS.stompV;
        s.score += comboPoints(s);
        events.push({ type: 'stomp', what: 'koopa' });
      } else if (damage(s, events, 'koopa')) {
        return true;
      }
    } else if (k.mode === 'shell') {
      if (k.kickLock > 0) continue;
      if (m.vy > 0 && mb.b - k.y < 12) {
        // 踩静止壳 → 停住
        k.vx = 0;
        k.revive = ITEMS.shellReviveTicks;
        m.vy = PHYSICS.stompV;
        s.score += 400;
        events.push({ type: 'stomp', what: 'shell' });
      } else {
        // 侧踢 → 顺着马里奥朝向滑出
        k.mode = 'slide';
        k.vx = ITEMS.shellSpeed * (m.vx >= 0 ? 1 : -1);
        k.revive = 0;
        k.kickLock = 12;
        s.score += 400;
        events.push({ type: 'shellkick' });
      }
    } else {
      // 滑行壳:踩它 = 停住并判定得分;侧面碰到 = 受伤
      if (m.vy > 0 && mb.b - k.y < 12) {
        k.mode = 'shell';
        k.vx = 0;
        k.revive = ITEMS.shellReviveTicks;
        m.vy = PHYSICS.stompV;
        s.score += comboPoints(s);
        events.push({ type: 'stomp', what: 'shell' });
      } else if (damage(s, events, 'shell')) {
        return true;
      }
    }
  }

  // 滑行壳撞死其他敌人(在 enemyCollisions 之后统一结算更稳,这里先做一次扫描)
  for (const k of s.koopas) {
    if (!k.alive || !k.active || k.mode !== 'slide') continue;
    for (const g of s.goombas) {
      if (!g.alive || !g.active) continue;
      if (overlap(k.x, k.y, 16, 14, g.x + 1, g.y + 2, 14, 14)) {
        g.alive = false;
        g.squashT = 30;
        s.score += 200;
        events.push({ type: 'stomp', what: 'goomba' });
      }
    }
    for (const o of s.koopas) {
      if (o === k || !o.alive || !o.active) continue;
      if (overlap(k.x, k.y, 16, 14, o.x + 1, o.y + 2, 14, 14)) {
        stompCombo(s, events);
        o.mode = 'shell';
        o.vx = 0;
        o.revive = ITEMS.shellReviveTicks;
        s.score += comboPoints(s);
        events.push({ type: 'stomp', what: 'koopa' });
      }
    }
  }
  void world;
  return false;
}

function stompCombo(s: GameState, events: GameEvent[]): void {
  s.combo += 1;
  if (s.combo > s.maxCombo) s.maxCombo = s.combo;
  if (s.combo > ITEMS.comboMax) s.combo = ITEMS.comboMax;
  void events;
}

function comboPoints(s: GameState): number {
  return ITEMS.comboBase * Math.max(1, s.combo);
}

// ---------------------------------------------------------------- 火球

function stepFireballs(world: World, s: GameState, events: GameEvent[]): void {
  for (const f of s.fireballs) {
    f.life -= 1;
    if (f.life <= 0) {
      f.life = 0;
      continue;
    }
    f.vy = Math.min(f.vy + PHYSICS.gravityFall * 0.8, PHYSICS.maxFall);
    const nx = f.x + f.vx;
    const col = Math.floor((f.vx > 0 ? nx + 7 : nx) / TILE);
    const row = Math.floor((f.y + 7) / TILE);
    if (isSolid(s.tiles[row * COLS + col] ?? 0)) {
      f.life = 0;
      continue;
    }
    f.x = nx;
    f.y += f.vy;
    // 触地弹跳(SMB 火球沿地弹行):不弹的话火球 30 tick 内就埋进地里,
    // 根本够不到远处的敌人。
    if (f.vy > 0) {
      const below = Math.floor((f.y + 8) / TILE);
      const bc0 = Math.floor((f.x + 1) / TILE);
      const bc1 = Math.floor((f.x + 6) / TILE);
      for (let c = bc0; c <= bc1; c += 1) {
        if (isSolid(s.tiles[below * COLS + c] ?? 0)) {
          f.y = below * TILE - 8;
          f.vy = -1.8;
          break;
        }
      }
    }
    if (f.y > ROWS * TILE + TILE) {
      f.life = 0;
      continue;
    }
    for (const g of s.goombas) {
      if (g.alive && g.active && overlap(f.x, f.y, 8, 8, g.x + 1, g.y + 2, 14, 14)) {
        g.alive = false;
        g.squashT = 30;
        s.score += 200;
        f.life = 0;
        events.push({ type: 'stomp', what: 'goomba' });
      }
    }
    for (const k of s.koopas) {
      if (k.alive && k.active && overlap(f.x, f.y, 8, 8, k.x + 1, k.y + 2, 14, 14)) {
        stompCombo(s, events);
        k.mode = 'shell';
        k.vx = 0;
        k.vy = 0;
        k.revive = ITEMS.shellReviveTicks;
        s.score += comboPoints(s);
        f.life = 0;
        events.push({ type: 'stomp', what: 'koopa' });
      }
    }
  }
  s.fireballs = s.fireballs.filter((f) => f.life > 0);
  void world;
}

// ---------------------------------------------------------------- 方块

function headHit(world: World, s: GameState, events: GameEvent[], col: number, row: number): void {
  const key = col * 32 + row;
  const tile = s.tiles[row * COLS + col] ?? 0;
  const b = s.blocks.get(key);

  if (tile === T_QBLOCK) {
    const item = b?.item ?? 'coin';
    if (item === 'mushroom' || item === 'flower') {
      s.items.push({
        id: s.tick * 10 + s.items.length,
        kind: item,
        // 站在 ?块**顶上**而不是块体内:否则道具一出生就踩在自己脚下的块上,
        // 永远走不下去(1-1 col16 的蘑菇会卡死在块顶)。
        x: col * TILE,
        y: row * TILE - TILE,
        vx: item === 'mushroom' ? ITEMS.mushroomSpeed : 0,
        vy: 0,
        emerge: 16,
        gone: false,
      });
      if (b) {
        b.used = true;
        b.bumpT = 8;
      }
      s.tiles[row * COLS + col] = T_USED;
      events.push({ type: 'bump', kind: 'q' });
      return;
    }
    // coin / multi:吐币(coin 一次即用,multi 连续)
    s.items.push({
      id: s.tick * 10 + s.items.length,
      kind: 'coin',
      x: col * TILE + 4,
      y: row * TILE,
      vx: 0.5,
      vy: -3.2,
      emerge: 0,
      gone: false,
    });
    s.coinCount += 1;
    s.score += 200;
    events.push({ type: 'coin', source: 'block' });
    events.push({ type: 'bump', kind: 'q' });
    if (item === 'multi' && b && b.coinsLeft > 1) {
      b.coinsLeft -= 1;
      b.bumpT = 8;
    } else if (b) {
      b.used = true;
      b.coinsLeft = 0;
      b.bumpT = 8;
      s.tiles[row * COLS + col] = T_USED;
    }
    return;
  }

  // 砖块:big/fire 顶碎,small 只顶一下
  if (tile !== T_EMPTY && s.mario.power !== 'small') {
    s.tiles[row * COLS + col] = T_EMPTY;
    s.blocks.delete(key);
    s.score += 50;
    events.push({ type: 'brickbreak' });
    return;
  }
  if (tile !== T_EMPTY) {
    if (b && !b.used) b.bumpT = 8;
    events.push({ type: 'bump', kind: 'brick' });
  }
  void world;
}

// ---------------------------------------------------------------- 死亡 / 重开

function die(s: GameState, events: GameEvent[], cause: DeathCause): void {
  s.phase = 'dead';
  s.deadTimer = DEAD_TICKS;
  s.combo = 0;
  s.deaths.push({ x: s.mario.x, cause });
  events.push({ type: 'death', cause });
}

function respawn(world: World, s: GameState, events: GameEvent[]): void {
  const d = world.data;
  s.attempts += 1;
  s.mario = newMario(d.spawn.col);
  s.goombas = spawnGoombas(world);
  s.koopas = spawnKoopas(world);
  s.plants = spawnPlants(world);
  s.items = [];
  s.fireballs = [];
  s.combo = 0;
  s.timeUnits = d.timeLimit;
  s.timeCarry = 0;
  s.camX = 0;
  s.phase = 'running';
  s.win = { stage: 'none', t: 0, slideFromY: 0 };
  events.push({ type: 'respawn' });
}

// ---------------------------------------------------------------- 通关序列

function startWinSequence(world: World, s: GameState, events: GameEvent[]): void {
  const m = s.mario;
  const bonusRows = Math.min(8, Math.max(1, GROUND_ROW - Math.floor((m.y + FOOT) / TILE)));
  const bonus = bonusRows * 100;
  s.score += bonus;
  s.phase = 'won';
  s.win = { stage: 'slide', t: 0, slideFromY: m.y };
  m.vx = 0;
  m.vy = 0;
  events.push({ type: 'flag' });
  events.push({ type: 'win', bonus });
  void world;
}

/** 旗杆下滑 → 落地 → 走向城堡 → 消失。phase 始终是 'won'(兼容既有断言)。 */
function stepWinSequence(world: World, s: GameState, _input: Input, events: GameEvent[]): void {
  const m = s.mario;
  s.win.t += 1;
  switch (s.win.stage) {
    case 'slide': {
      m.x = world.data.flagCol * TILE + 6 - OX - BW + 2;
      m.y = Math.min(s.win.slideFromY, (GROUND_ROW - 1) * TILE);
      if (m.y >= (GROUND_ROW - 1) * TILE || s.win.t >= SLIDE_TICKS) {
        m.y = (GROUND_ROW - 1) * TILE;
        m.face = 1;
        s.win = { stage: 'walk', t: 0, slideFromY: 0 };
      }
      break;
    }
    case 'walk': {
      m.vx = PHYSICS.maxWalk;
      m.x += m.vx;
      m.face = 1;
      const castleX = world.data.castleCol * TILE;
      if (m.x >= castleX + 8) {
        m.hidden = true;
        s.win = { stage: 'enter', t: 0, slideFromY: 0 };
      } else if (s.win.t >= CASTLE_TICKS) {
        m.hidden = true;
        s.win = { stage: 'enter', t: 0, slideFromY: 0 };
      }
      break;
    }
    case 'enter': {
      if (s.win.t === 1) {
        // 时间奖励结算
        s.timeBonus = Math.max(0, s.timeUnits) * ITEMS.timeBonusPerUnit;
        s.score += s.timeBonus;
      }
      break;
    }
    case 'none':
      break;
  }
  void events;
}

// ---------------------------------------------------------------- 查询辅助

/** 供 UI/测试读取的实心判定(走本局副本)。 */
export function solidAtPx(s: GameState, px: number, py: number): boolean {
  return isSolid(tileAt({ ...WORLD_1_1, tiles: s.tiles }, px, py));
}

/** 状态指纹(测试锁定确定性;遥测里做 stale 守卫的辅助)。 */
export function stateHash(s: GameState): string {
  return JSON.stringify({
    t: s.tick,
    p: s.phase,
    m: [s.mario.x, s.mario.y, s.mario.vx, s.mario.vy, s.mario.power],
    s: s.score,
    c: s.coinCount,
    g: s.goombas.filter((g) => g.alive).map((g) => [g.x, g.y]),
    k: s.koopas.filter((k) => k.alive).map((k) => [k.x, k.y, k.mode]),
    i: s.items.map((i) => [i.kind, i.x, i.y]),
  });
}
