/**
 * 超级马里奥 1-1 · 关卡数据与物理常量(docs/11 §3)。
 *
 * 设计要点:
 * - 世界 = 16px 瓦片网格,224 列 × 15 行;地面表面在 row 13(13、14 两行是地面)。
 * - **零二进制资源**:瓦片/布局全部由本文件的声明式数据生成引擎用的 Uint8Array。
 * - 物理常量集中于此,planner 的跳跃可达表(JUMP)由同一组常量推导并单测锁定——
 *   保证"规划器认为能跳的,引擎一定跳得过"。
 * - v1 实体集:Goomba / 金币 / ?块(出币)/ 砖块(仅顶动)/ 管道 / 坑 / 旗杆。
 *   明确砍掉(见 docs/11 §3.2):蘑菇、Koopa、地下房、Warp。
 */

export const TILE = 16;
export const ROWS = 15;
export const COLS = 224;
/** 地面表面行(实体站立面的行号);地面填充 13、14 两行。 */
export const GROUND_ROW = 13;

/** 物理常量(单位:px/tick,60Hz 固定步长)。手感基准:人玩通关且成立(M1 闸门)。 */
export const PHYSICS = {
  walkAcc: 0.08,
  runAcc: 0.12,
  friction: 0.1,
  maxWalk: 1.4,
  maxRun: 2.4,
  gravityUp: 0.24,
  /** 松开跳跃键的截断重力(变高跳:松手 = 短跳)。 */
  gravityCut: 0.5,
  gravityFall: 0.34,
  jumpV: -5.9,
  stompV: -3.4,
  maxFall: 6,
  enemySpeed: 0.35,
} as const;

/** 跳跃可达表:由物理常量推导,planner 与测试共用同一份数据。 */
function deriveJump(p: typeof PHYSICS): {
  tUp: number;
  tDown: number;
  airtime: number;
  heightPx: number;
  heightTiles: number;
  walkDistTiles: number;
  runDistTiles: number;
} {
  const tUp = Math.ceil(-p.jumpV / p.gravityUp);
  const heightPx = (-p.jumpV) ** 2 / (2 * p.gravityUp);
  const tDown = Math.ceil(Math.sqrt((2 * heightPx) / p.gravityFall));
  const airtime = tUp + tDown;
  return {
    tUp,
    tDown,
    airtime,
    heightPx,
    heightTiles: Math.floor(heightPx / TILE),
    walkDistTiles: Math.floor((p.maxWalk * airtime) / TILE),
    runDistTiles: Math.floor((p.maxRun * airtime) / TILE),
  };
}
export const JUMP = deriveJump(PHYSICS);

/** 瓦片编码。 */
export const T_EMPTY = 0;
export const T_GROUND = 1;
export const T_BRICK = 2;
export const T_QBLOCK = 3;
export const T_USED = 4;
export const T_PIPE = 5;
export const T_STAIR = 6;
export const SOLID = new Set([T_GROUND, T_BRICK, T_QBLOCK, T_USED, T_PIPE, T_STAIR]);

export type GroundSpan = { startCol: number; endCol: number };
export type PipeDef = { col: number; height: number };
export type BlockDef = { col: number; row: number };
export type CoinDef = { col: number; row: number };
export type GoombaDef = { col: number; row: number };
export type KoopaDef = { col: number; row: number };
/** ?块产出物:coin=一枚币;multi=连续吐币(踩一次出 8 枚,限时变 used);mushroom/flower=道具。 */
export type QBlockItem = 'coin' | 'multi' | 'mushroom' | 'flower';
export type QBlockDef = BlockDef & { item: QBlockItem };
/** 食人花:绑定到某根管道(col 为管道左列),沿管口上下伸缩。 */
export type PlantDef = { col: number };

export type WorldData = {
  groundSpans: GroundSpan[];
  pipes: PipeDef[];
  bricks: BlockDef[];
  qblocks: QBlockDef[];
  coins: CoinDef[];
  goombas: GoombaDef[];
  /** 乌龟(踩一下变壳,踢出去会滑行撞死其他敌人)。 */
  koopas: KoopaDef[];
  /** 食人花(从管口升降;马里奥贴近管子时不伸出)。 */
  plants: PlantDef[];
  stairs: Array<{ col: number; height: number }>; // 单列台阶:占据 rows (13-height)..12
  flagCol: number;
  /** 城堡左墙列(旗杆之后,通关走入即消失)。 */
  castleCol: number;
  spawn: { col: number };
  timeLimit: number;
  name: string;
};

/** 道具/敌人共用常量(与 PHYSICS 同源风格,集中一处便于单测锁定)。 */
export const ITEMS = {
  /** 蘑菇水平速度 px/tick(比 Goomba 慢,便于追)。 */
  mushroomSpeed: 0.6,
  /** 食人花伸出/缩回一个行程的 tick 数。 */
  plantExtendTicks: 36,
  plantHoldTicks: 48,
  plantRetractTicks: 24,
  /** 龟壳静止复活倒计时 tick(踩/踢后开始计时,归零复活)。 */
  shellReviveTicks: 420,
  /** 龟壳被踢后的滑行速度。 */
  shellSpeed: 3.2,
  /** 受伤后的无敌闪烁 tick。 */
  invulnTicks: 96,
  /** 变身(变大/变小)定格 tick。 */
  growFreezeTicks: 40,
  /** 连踩连击每级的加成基数(第 n 连 = COMBO_BASE × n)。 */
  comboBase: 100,
  comboMax: 8,
  /** 旗杆下滑到底按时间奖励:每单位时间 = 50 分。 */
  timeBonusPerUnit: 50,
} as const;

/** 1-1 布局(致敬复刻,自绘数据;坑 3 处:2/3/2 宽,均 ≤ RUN 跳距)。 */
export const LEVEL_1_1: WorldData = {
  name: '1-1',
  groundSpans: [
    { startCol: 0, endCol: 68 },
    { startCol: 71, endCol: 85 },
    { startCol: 89, endCol: 152 },
    { startCol: 155, endCol: 222 },
  ],
  pipes: [
    { col: 28, height: 2 },
    { col: 38, height: 3 },
    { col: 46, height: 4 },
    { col: 57, height: 4 },
  ],
  bricks: [
    { col: 20, row: 9 },
    { col: 22, row: 9 },
    { col: 24, row: 9 },
    { col: 77, row: 9 },
    { col: 79, row: 9 },
    { col: 100, row: 9 },
    { col: 101, row: 9 },
    { col: 112, row: 9 },
    { col: 118, row: 5 },
    { col: 119, row: 5 },
  ],
  qblocks: [
    { col: 16, row: 9, item: 'mushroom' },
    { col: 21, row: 9, item: 'coin' },
    { col: 23, row: 9, item: 'multi' },
    { col: 22, row: 5, item: 'multi' },
    { col: 78, row: 5, item: 'coin' },
    { col: 94, row: 9, item: 'flower' },
    { col: 106, row: 9, item: 'multi' },
    { col: 109, row: 9, item: 'coin' },
  ],
  coins: [
    { col: 10, row: 12 },
    { col: 12, row: 12 },
    { col: 69, row: 10 },
    { col: 70, row: 10 },
    { col: 87, row: 9 },
    { col: 96, row: 8 },
    { col: 97, row: 8 },
    { col: 122, row: 11 },
    { col: 123, row: 11 },
    { col: 124, row: 11 },
    { col: 153, row: 10 },
    { col: 154, row: 10 },
  ],
  goombas: [
    { col: 22, row: 12 },
    { col: 40, row: 12 },
    { col: 52, row: 12 },
    { col: 60, row: 12 },
    { col: 81, row: 12 },
    { col: 97, row: 12 },
    { col: 99, row: 12 },
    { col: 114, row: 12 },
    { col: 126, row: 12 },
    { col: 146, row: 12 },
    { col: 161, row: 12 },
    { col: 182, row: 12 },
    { col: 190, row: 12 },
  ],
  koopas: [
    { col: 44, row: 12 },
    { col: 92, row: 12 },
    { col: 158, row: 12 },
    { col: 194, row: 12 },
  ],
  plants: [{ col: 46 }, { col: 57 }],
  stairs: [
    // 中段:上 4 级 + 下 4 级(峰顶 2 瓦宽)
    { col: 134, height: 1 },
    { col: 135, height: 2 },
    { col: 136, height: 3 },
    { col: 137, height: 4 },
    { col: 138, height: 4 },
    { col: 139, height: 3 },
    { col: 140, height: 2 },
    { col: 141, height: 1 },
    // 终段:8 级长阶,旗杆前
    { col: 168, height: 1 },
    { col: 169, height: 2 },
    { col: 170, height: 3 },
    { col: 171, height: 4 },
    { col: 172, height: 5 },
    { col: 173, height: 6 },
    { col: 174, height: 7 },
    { col: 175, height: 8 },
    // 旗杆座
    { col: 198, height: 1 },
  ],
  flagCol: 198,
  castleCol: 202,
  spawn: { col: 3 },
  timeLimit: 300,
};

/**
 * 1-2(第二关,全新布局):教学→组合→收尾三段式。
 * 相对 1-1 的变化:坑更密(4 处)、连续高低管道、食人花 3 根、
 * 空中浮砖带、乌龟 ×3、末段双阶 + 旗杆。
 */
export const LEVEL_1_2: WorldData = {
  name: '1-2',
  groundSpans: [
    { startCol: 0, endCol: 40 },
    { startCol: 44, endCol: 62 },
    { startCol: 65, endCol: 77 },
    { startCol: 80, endCol: 108 },
    { startCol: 111, endCol: 145 },
    { startCol: 148, endCol: 222 },
  ],
  pipes: [
    { col: 20, height: 2 },
    { col: 24, height: 3 },
    { col: 29, height: 4 },
    { col: 33, height: 2 },
    { col: 52, height: 3 },
    { col: 70, height: 4 },
    { col: 88, height: 2 },
    { col: 91, height: 3 },
    { col: 94, height: 4 },
    { col: 119, height: 4 },
    { col: 122, height: 2 },
    { col: 130, height: 3 },
  ],
  bricks: [
    { col: 8, row: 9 },
    { col: 10, row: 9 },
    { col: 12, row: 9 },
    { col: 11, row: 5 },
    { col: 34, row: 9 },
    { col: 36, row: 9 },
    { col: 38, row: 9 },
    { col: 96, row: 5 },
    { col: 98, row: 5 },
    { col: 100, row: 5 },
    { col: 102, row: 5 },
    { col: 114, row: 9 },
    { col: 116, row: 9 },
    { col: 136, row: 9 },
    { col: 138, row: 9 },
  ],
  qblocks: [
    { col: 9, row: 9, item: 'mushroom' },
    { col: 11, row: 9, item: 'coin' },
    { col: 12, row: 5, item: 'multi' },
    { col: 37, row: 9, item: 'flower' },
    { col: 56, row: 9, item: 'multi' },
    { col: 58, row: 9, item: 'coin' },
    { col: 97, row: 9, item: 'mushroom' },
    { col: 99, row: 9, item: 'coin' },
    { col: 100, row: 5, item: 'multi' },
    { col: 117, row: 9, item: 'coin' },
    { col: 139, row: 9, item: 'mushroom' },
    { col: 160, row: 9, item: 'multi' },
    { col: 162, row: 9, item: 'coin' },
  ],
  coins: [
    { col: 5, row: 12 },
    { col: 7, row: 12 },
    { col: 41, row: 11 },
    { col: 42, row: 10 },
    { col: 63, row: 11 },
    { col: 64, row: 10 },
    { col: 78, row: 11 },
    { col: 109, row: 10 },
    { col: 110, row: 9 },
    { col: 146, row: 11 },
    { col: 147, row: 10 },
    { col: 168, row: 9 },
    { col: 169, row: 9 },
    { col: 186, row: 8 },
  ],
  goombas: [
    { col: 14, row: 12 },
    { col: 36, row: 12 },
    { col: 50, row: 12 },
    { col: 74, row: 12 },
    { col: 84, row: 12 },
    { col: 104, row: 12 },
    { col: 126, row: 12 },
    { col: 152, row: 12 },
    { col: 178, row: 12 },
    { col: 196, row: 12 },
  ],
  koopas: [
    { col: 18, row: 12 },
    { col: 58, row: 12 },
    { col: 106, row: 12 },
    { col: 140, row: 12 },
    { col: 186, row: 12 },
  ],
  plants: [{ col: 29 }, { col: 70 }, { col: 94 }],
  stairs: [
    { col: 152, height: 1 },
    { col: 153, height: 2 },
    { col: 154, height: 3 },
    { col: 155, height: 4 },
    { col: 156, height: 3 },
    { col: 157, height: 2 },
    { col: 158, height: 1 },
    { col: 170, height: 1 },
    { col: 171, height: 2 },
    { col: 172, height: 3 },
    { col: 173, height: 4 },
    { col: 174, height: 5 },
    { col: 175, height: 6 },
    { col: 176, height: 4 },
    { col: 177, height: 2 },
    { col: 178, height: 1 },
    { col: 198, height: 1 },
  ],
  flagCol: 198,
  castleCol: 202,
  spawn: { col: 3 },
  timeLimit: 320,
};

/** 关卡注册表(GamesPage 下拉用)。 */
export const LEVELS: Record<string, WorldData> = {
  '1-1': LEVEL_1_1,
  '1-2': LEVEL_1_2,
};
export const LEVEL_KEYS = Object.keys(LEVELS) as Array<keyof typeof LEVELS>;

export type World = {
  data: WorldData;
  tiles: Uint8Array; // COLS*ROWS
  gaps: Array<{ startCol: number; endCol: number }>; // 由 groundSpans 推导的坑
  worldWidthPx: number;
};

function buildWorld(data: WorldData): World {
  const tiles = new Uint8Array(COLS * ROWS);
  const set = (col: number, row: number, v: number): void => {
    if (col >= 0 && col < COLS && row >= 0 && row < ROWS) tiles[row * COLS + col] = v;
  };
  for (const s of data.groundSpans)
    for (let c = s.startCol; c <= s.endCol; c += 1) {
      set(c, GROUND_ROW, T_GROUND);
      set(c, GROUND_ROW + 1, T_GROUND);
    }
  for (const p of data.pipes)
    for (let i = 0; i < 2; i += 1)
      for (let r = GROUND_ROW - p.height; r < GROUND_ROW; r += 1) set(p.col + i, r, T_PIPE);
  for (const b of data.bricks) set(b.col, b.row, T_BRICK);
  for (const q of data.qblocks) set(q.col, q.row, T_QBLOCK);
  for (const s of data.stairs)
    for (let r = GROUND_ROW - s.height; r < GROUND_ROW; r += 1) set(s.col, r, T_STAIR);
  // 世界右缘墙(旗杆之后)
  for (let r = 0; r < ROWS; r += 1) set(COLS - 1, r, T_GROUND);
  // 坑 = 相邻地面段之间的空隙
  const gaps: Array<{ startCol: number; endCol: number }> = [];
  const spans = [...data.groundSpans].sort((a, b) => a.startCol - b.startCol);
  for (let i = 0; i < spans.length - 1; i += 1) {
    const a = spans[i];
    const b = spans[i + 1];
    if (a && b && b.startCol > a.endCol + 1)
      gaps.push({ startCol: a.endCol + 1, endCol: b.startCol - 1 });
  }
  return { data, tiles, gaps, worldWidthPx: COLS * TILE };
}

export const WORLD_1_1: World = buildWorld(LEVEL_1_1);
export const WORLD_1_2: World = buildWorld(LEVEL_1_2);

/** 关卡 key → World(GamesPage 切关用;未知 key 回退 1-1)。 */
export const WORLDS: Record<string, World> = { '1-1': WORLD_1_1, '1-2': WORLD_1_2 };
export function getWorld(key: string): World {
  return WORLDS[key] ?? WORLD_1_1;
}

/** 某根管子的管口行(食人花升降的基准);非管道列回 GROUND_ROW。 */
export function pipeTopRow(data: WorldData, col: number): number {
  const p = data.pipes.find((q) => col === q.col || col === q.col + 1);
  return p ? GROUND_ROW - p.height : GROUND_ROW;
}

/** 该列是否有管道(食人花才可生根)。 */
export function isPipeCol(data: WorldData, col: number): boolean {
  return data.pipes.some((q) => col === q.col || col === q.col + 1);
}

export function tileAt(world: World, px: number, py: number): number {
  const col = Math.floor(px / TILE);
  const row = Math.floor(py / TILE);
  if (col < 0 || col >= COLS) return T_GROUND; // 世界左右缘视为墙
  if (row < 0 || row >= ROWS) return T_EMPTY;
  return world.tiles[row * COLS + col] ?? T_EMPTY;
}

export function isSolid(tile: number): boolean {
  return SOLID.has(tile);
}
