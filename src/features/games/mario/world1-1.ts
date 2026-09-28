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

export type WorldData = {
  groundSpans: GroundSpan[];
  pipes: PipeDef[];
  bricks: BlockDef[];
  qblocks: BlockDef[];
  coins: CoinDef[];
  goombas: GoombaDef[];
  stairs: Array<{ col: number; height: number }>; // 单列台阶:占据 rows (13-height)..12
  flagCol: number;
  spawn: { col: number };
  timeLimit: number;
};

/** 1-1 布局(致敬复刻,自绘数据;坑 3 处:2/3/2 宽,均 ≤ RUN 跳距)。 */
export const LEVEL_1_1: WorldData = {
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
    { col: 16, row: 9 },
    { col: 21, row: 9 },
    { col: 23, row: 9 },
    { col: 22, row: 5 },
    { col: 78, row: 5 },
    { col: 94, row: 9 },
    { col: 106, row: 9 },
    { col: 109, row: 9 },
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
  spawn: { col: 3 },
  timeLimit: 300,
};

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
