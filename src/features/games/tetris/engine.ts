/**
 * Tetris 引擎(System 2 求解器的棋盘侧):纯函数、无依赖、可单测。
 *
 * 棋盘:Uint8Array(ROWS*COLS),值 0=空,1..7=锁定方块的 piece id。
 * 形状:每 piece 4 个旋转态的 cell offsets(3×3 / I 用 4×4 / O 恒定)。
 * 评估:el-Tetris 权重的 Dellacherie 特征集(消行/行 transitions/列
 * transitions/洞/井深)——经典解算器,单次枚举 <1ms,与 JEV Lab 的
 * "plan score · 0.2ms" 同一档。
 */

export const COLS = 10;
export const ROWS = 20;

export type Board = Uint8Array;

/** 1=I 2=O 3=T 4=S 5=Z 6=J 7=L(下标对应 piece id-1)。 */
export const PIECE_NAMES = ['I', 'O', 'T', 'S', 'Z', 'J', 'L'] as const;
export type PieceName = (typeof PIECE_NAMES)[number];

export const PIECE_IDS: Record<PieceName, number> = {
  I: 1,
  O: 2,
  T: 3,
  S: 4,
  Z: 5,
  J: 6,
  L: 7,
};

/** PIECES[pieceId-1][rot][cell] = [dx,dy](piece 局部坐标,x 向右 y 向下)。 */
export const PIECES: number[][][][] = [
  // I(4×4)
  [
    [
      [0, 1],
      [1, 1],
      [2, 1],
      [3, 1],
    ],
    [
      [2, 0],
      [2, 1],
      [2, 2],
      [2, 3],
    ],
    [
      [0, 2],
      [1, 2],
      [2, 2],
      [3, 2],
    ],
    [
      [1, 0],
      [1, 1],
      [1, 2],
      [1, 3],
    ],
  ],
  // O(恒定)
  [
    [
      [1, 0],
      [2, 0],
      [1, 1],
      [2, 1],
    ],
    [
      [1, 0],
      [2, 0],
      [1, 1],
      [2, 1],
    ],
    [
      [1, 0],
      [2, 0],
      [1, 1],
      [2, 1],
    ],
    [
      [1, 0],
      [2, 0],
      [1, 1],
      [2, 1],
    ],
  ],
  // T
  [
    [
      [1, 0],
      [0, 1],
      [1, 1],
      [2, 1],
    ],
    [
      [1, 0],
      [1, 1],
      [2, 1],
      [1, 2],
    ],
    [
      [0, 1],
      [1, 1],
      [2, 1],
      [1, 2],
    ],
    [
      [1, 0],
      [0, 1],
      [1, 1],
      [1, 2],
    ],
  ],
  // S
  [
    [
      [1, 0],
      [2, 0],
      [0, 1],
      [1, 1],
    ],
    [
      [1, 0],
      [1, 1],
      [2, 1],
      [2, 2],
    ],
    [
      [1, 1],
      [2, 1],
      [0, 2],
      [1, 2],
    ],
    [
      [0, 0],
      [0, 1],
      [1, 1],
      [1, 2],
    ],
  ],
  // Z
  [
    [
      [0, 0],
      [1, 0],
      [1, 1],
      [2, 1],
    ],
    [
      [2, 0],
      [1, 1],
      [2, 1],
      [1, 2],
    ],
    [
      [0, 1],
      [1, 1],
      [1, 2],
      [2, 2],
    ],
    [
      [1, 0],
      [0, 1],
      [1, 1],
      [0, 2],
    ],
  ],
  // J
  [
    [
      [0, 0],
      [0, 1],
      [1, 1],
      [2, 1],
    ],
    [
      [1, 0],
      [2, 0],
      [1, 1],
      [1, 2],
    ],
    [
      [0, 1],
      [1, 1],
      [2, 1],
      [2, 2],
    ],
    [
      [1, 0],
      [1, 1],
      [0, 2],
      [1, 2],
    ],
  ],
  // L
  [
    [
      [2, 0],
      [0, 1],
      [1, 1],
      [2, 1],
    ],
    [
      [1, 0],
      [1, 1],
      [1, 2],
      [2, 2],
    ],
    [
      [0, 1],
      [1, 1],
      [2, 1],
      [0, 2],
    ],
    [
      [0, 0],
      [1, 0],
      [1, 1],
      [1, 2],
    ],
  ],
];

export function newBoard(): Board {
  return new Uint8Array(ROWS * COLS);
}

export function cloneBoard(board: Board): Board {
  return board.slice();
}

export function pieceName(id: number): PieceName {
  return PIECE_NAMES[Math.min(7, Math.max(1, id)) - 1] ?? 'T';
}

export function cellsOf(pieceId: number, rot: number): Array<[number, number]> {
  const byPiece = PIECES[pieceId - 1] ?? PIECES[2];
  const cells = (byPiece ?? [])[((rot % 4) + 4) % 4];
  return (cells ?? []) as Array<[number, number]>;
}

export function collides(
  board: Board,
  pieceId: number,
  rot: number,
  px: number,
  py: number,
): boolean {
  for (const [dx, dy] of cellsOf(pieceId, rot)) {
    const x = px + dx;
    const y = py + dy;
    if (x < 0 || x >= COLS || y < 0 || y >= ROWS) return true;
    if (board[y * COLS + x] !== 0) return true;
  }
  return false;
}

/** 原地锁定(调用方保证不碰撞)。 */
export function lockPiece(
  board: Board,
  pieceId: number,
  rot: number,
  px: number,
  py: number,
): void {
  for (const [dx, dy] of cellsOf(pieceId, rot)) {
    board[(py + dy) * COLS + (px + dx)] = pieceId;
  }
}

/** 消行:返回新棋盘与消除行数(不可变,便于渲染动画扩展)。 */
export function clearLines(board: Board): { board: Board; cleared: number } {
  const out = newBoard();
  let write = ROWS - 1;
  let cleared = 0;
  for (let y = ROWS - 1; y >= 0; y -= 1) {
    let full = true;
    for (let x = 0; x < COLS; x += 1) {
      if (board[y * COLS + x] === 0) {
        full = false;
        break;
      }
    }
    if (!full) {
      for (let x = 0; x < COLS; x += 1) out[write * COLS + x] = board[y * COLS + x] ?? 0;
      write -= 1;
    } else {
      cleared += 1;
    }
  }
  return { board: out, cleared };
}

/** 硬降落点:给定 rot/x 的最终 y(不修改棋盘)。越界非法返回 -1。 */
export function dropY(board: Board, pieceId: number, rot: number, px: number): number {
  let y = 0;
  if (collides(board, pieceId, rot, px, y)) return -1;
  while (!collides(board, pieceId, rot, px, y + 1)) y += 1;
  return y;
}

// ---- Dellacherie 特征(el-Tetris 权重) ----

export type Features = {
  aggHeight: number;
  cleared: number;
  rowTransitions: number;
  colTransitions: number;
  holes: number;
  wellSums: number;
};

/** 对"锁定+消行之后"的棋盘评估(消行数由调用方传入)。 */
export function features(board: Board, cleared: number): Features {
  const cell = (x: number, y: number): number => board[y * COLS + x] ?? 0;
  const filled = (x: number, y: number): number =>
    x < 0 || x >= COLS || y >= ROWS ? 1 : y < 0 ? 0 : cell(x, y) === 0 ? 0 : 1;

  // 每列高度
  const heights = new Array<number>(COLS).fill(0);
  for (let x = 0; x < COLS; x += 1) {
    for (let y = 0; y < ROWS; y += 1) {
      if (cell(x, y) !== 0) {
        heights[x] = ROWS - y;
        break;
      }
    }
  }
  const aggHeight = heights.reduce((a, b) => a + b, 0);

  // 洞:格子上方存在实块而自身为空
  let holes = 0;
  for (let x = 0; x < COLS; x += 1) {
    let seen = false;
    for (let y = 0; y < ROWS; y += 1) {
      if (cell(x, y) !== 0) seen = true;
      else if (seen) holes += 1;
    }
  }

  // 行 transitions:一行内 实↔空 的切换次数(两端视作墙)
  let rowTransitions = 0;
  for (let y = 0; y < ROWS; y += 1) {
    let prev = 1;
    for (let x = 0; x < COLS; x += 1) {
      const cur = cell(x, y) === 0 ? 0 : 1;
      if (cur !== prev) rowTransitions += 1;
      prev = cur;
    }
    if (prev !== 1) rowTransitions += 1;
  }

  // 列 transitions(顶部以上视作空,底部以下视作墙)
  let colTransitions = 0;
  for (let x = 0; x < COLS; x += 1) {
    let prev = 0;
    for (let y = 0; y < ROWS; y += 1) {
      const cur = cell(x, y) === 0 ? 0 : 1;
      if (cur !== prev) colTransitions += 1;
      prev = cur;
    }
    if (prev !== 1) colTransitions += 1;
  }

  // 井深和:两侧都比它高的空列深度累加
  let wellSums = 0;
  for (let x = 0; x < COLS; x += 1) {
    for (let y = 0; y < ROWS; y += 1) {
      if (cell(x, y) !== 0) break;
      const left = filled(x - 1, y);
      const right = filled(x + 1, y);
      if (left === 1 && right === 1) {
        let depth = 1;
        let yy = y + 1;
        while (yy < ROWS && cell(x, yy) === 0) {
          depth += 1;
          yy += 1;
        }
        wellSums += depth;
        break;
      }
    }
  }

  return { aggHeight, cleared, rowTransitions, colTransitions, holes, wellSums };
}

/** el-Tetris 公开权重(免训练的强基线,和 JEV Lab 的 plan score 同源思路)。 */
const W = {
  landingHeight: -4.5,
  cleared: 3.4181268101392694,
  rowTransitions: -3.2178882868487753,
  colTransitions: -9.348695305445199,
  holes: -7.899265427351652,
  wellSums: -3.3855972247263626,
};

/** 一次放置的总评分(越大越好)。 */
export function evaluatePlacement(
  boardBefore: Board,
  pieceId: number,
  rot: number,
  px: number,
): number {
  const y = dropY(boardBefore, pieceId, rot, px);
  if (y < 0) return Number.NEGATIVE_INFINITY;
  const next = cloneBoard(boardBefore);
  lockPiece(next, pieceId, rot, px, y);
  const { board: locked, cleared } = clearLines(next);
  const f = features(locked, cleared);
  const landingHeight = ROWS - y;
  return (
    W.landingHeight * landingHeight +
    W.cleared * cleared +
    W.rowTransitions * f.rowTransitions +
    W.colTransitions * f.colTransitions +
    W.holes * f.holes +
    W.wellSums * f.wellSums
  );
}

export type Placement = {
  rot: number;
  x: number;
  y: number;
  score: number;
  cleared: number;
};

/** System 2:枚举全部 rot×x,按评分降序(带结果克隆,供直接执行)。 */
export function solvePlacements(board: Board, pieceId: number): Placement[] {
  const out: Placement[] = [];
  for (let rot = 0; rot < 4; rot += 1) {
    for (let x = -2; x <= COLS; x += 1) {
      const y = dropY(board, pieceId, rot, x);
      if (y < 0) continue;
      const next = cloneBoard(board);
      lockPiece(next, pieceId, rot, x, y);
      const { board: locked, cleared } = clearLines(next);
      const f = features(locked, cleared);
      const landingHeight = ROWS - y;
      const score =
        W.landingHeight * landingHeight +
        W.cleared * cleared +
        W.rowTransitions * f.rowTransitions +
        W.colTransitions * f.colTransitions +
        W.holes * f.holes +
        W.wellSums * f.wellSums;
      out.push({ rot, x, y, score, cleared });
    }
  }
  out.sort((a, b) => b.score - a.score);
  return out;
}

/**
 * System 2 进阶:2-block lookahead。
 * 对当前块每个合法放置,先模拟落定,再用下一块的 1-block 最优评分评价该局面——
 * 「这步放完后,下一块最好能怎样」。修复单块贪心在后期堆高时的死局倾向
 * (本次留 1 格缝、下次填不上)。当前块自身评分作为 tie-break 加入,避免
 * 「两步幻想」:为下一块的高分牺牲眼前(消行仍按两块累计计)。
 * 性能:≤40×≤40 次特征评估,JS 每次调用 <10ms,仅在新块出生时调用一次。
 */
export function solvePlacements2(board: Board, pieceId: number, nextPieceId: number): Placement[] {
  const first = solvePlacements(board, pieceId);
  if (first.length === 0) return first;
  const scored: Placement[] = [];
  for (const p of first) {
    // 复现该放置落定后的盘面
    const y = dropY(board, pieceId, p.rot, p.x);
    if (y < 0) continue;
    const mid = cloneBoard(board);
    lockPiece(mid, pieceId, p.rot, p.x, y);
    const { board: after, cleared } = clearLines(mid);
    // 下一块的最优 1-block 评分(无合法放置 = -∞,该分支视为死路)
    const nextBest = solvePlacements(after, nextPieceId)[0];
    const nextScore = nextBest?.score ?? Number.NEGATIVE_INFINITY;
    scored.push({
      ...p,
      score: nextScore + p.score * 0.5 + W.cleared * cleared * 2,
    });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored;
}

/** 当前堆高(最高列)。 */
export function stackHeight(board: Board): number {
  for (let y = 0; y < ROWS; y += 1) {
    for (let x = 0; x < COLS; x += 1) {
      if (board[y * COLS + x] !== 0) return ROWS - y;
    }
  }
  return 0;
}

/** 棋盘文本化(给 Laya 的 state):'.'=空 '#'=实块,顶部行在前。 */
export function boardToText(board: Board): string[] {
  const rows: string[] = [];
  for (let y = 0; y < ROWS; y += 1) {
    let line = '';
    for (let x = 0; x < COLS; x += 1) line += board[y * COLS + x] === 0 ? '.' : '#';
    rows.push(line);
  }
  return rows;
}

/** 7-bag 随机器(现代俄罗斯方块的公平发牌)。 */
export function makeBag(rng: () => number = Math.random): () => number {
  let queue: number[] = [];
  return () => {
    if (queue.length === 0) {
      queue = [1, 2, 3, 4, 5, 6, 7];
      for (let i = queue.length - 1; i > 0; i -= 1) {
        const j = Math.floor(rng() * (i + 1));
        const a = queue[i] ?? 0;
        const b = queue[j] ?? 0;
        queue[i] = b;
        queue[j] = a;
      }
    }
    return queue.shift() as number;
  };
}
