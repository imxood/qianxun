import { describe, expect, it } from 'vitest';

import {
  PIECE_IDS,
  clearLines,
  collides,
  dropY,
  features,
  lockPiece,
  makeBag,
  newBoard,
  solvePlacements,
  solvePlacements2,
  stackHeight,
} from './engine';

describe('tetris engine', () => {
  it('空板出生位不碰撞,越界与重叠检出', () => {
    const b = newBoard();
    expect(collides(b, PIECE_IDS.T, 0, 3, 0)).toBe(false);
    expect(collides(b, PIECE_IDS.T, 0, -1, 0)).toBe(true);
    expect(collides(b, PIECE_IDS.T, 0, 3, 19)).toBe(true);
    // O 的出生 x=4;T 放 x=8 会越界(右缘)
    expect(collides(b, PIECE_IDS.T, 0, 8, 0)).toBe(true);
  });

  it('dropY 落到堆顶或板底', () => {
    const b = newBoard();
    expect(dropY(b, PIECE_IDS.O, 0, 4)).toBe(18);
    // 中间竖一堵 10 高的墙(x=4、5)
    for (let y = 10; y < 20; y += 1) {
      b[y * 10 + 4] = PIECE_IDS.I;
      b[y * 10 + 5] = PIECE_IDS.I;
    }
    // O 在 x=4 占列 5、6,与 x=4、5 的墙相交:底行(dy=1)最高能到 y=9,
    // 但 y=9 时 cells 触到 y=10 的墙 → 落点 y=8
    expect(dropY(b, PIECE_IDS.O, 0, 4)).toBe(8);
  });

  it('锁定后整行清除', () => {
    const b = newBoard();
    for (let x = 0; x < 10; x += 1) {
      if (x !== 0) b[19 * 10 + x] = PIECE_IDS.I;
    }
    lockPiece(b, PIECE_IDS.I, 0, 0, 18); // I 横放(cells 落在 y=19)补最后一行
    const { cleared } = clearLines(b);
    expect(cleared).toBe(1);
  });

  it('特征:洞与堆高与 bumpiness', () => {
    const b = newBoard();
    // 左列堆 5 高,但 y=17 留洞:先填 15..19,再挖 y=17
    for (let y = 15; y < 20; y += 1) b[y * 10 + 0] = PIECE_IDS.I;
    b[17 * 10 + 0] = 0;
    // 右列堆 2 高
    for (let y = 18; y < 20; y += 1) b[y * 10 + 9] = PIECE_IDS.I;
    const f = features(b, 0);
    expect(f.holes).toBe(1);
    expect(stackHeight(b)).toBe(5);
    // bumpiness 只有一对相邻列:|5-0|+|0-…0|+…+|0-2| = 5+2
    expect(f.aggHeight).toBe(7);
  });

  it('求解器:空板 O 块的最优解不制造洞,且排序降序', () => {
    const b = newBoard();
    const placements = solvePlacements(b, PIECE_IDS.O);
    expect(placements.length).toBeGreaterThan(0);
    for (let i = 1; i < placements.length; i += 1) {
      expect(placements[i - 1]?.score).toBeGreaterThanOrEqual(placements[i]?.score ?? 0);
    }
    // 把最优解放进空板:不应产生洞
    const top = placements[0];
    expect(top).toBeDefined();
    if (!top) return;
    const next = b.slice();
    lockPiece(next, PIECE_IDS.O, top.rot, top.x, top.y);
    expect(features(next, 0).holes).toBe(0);
  });

  it('求解器:有洞的板面,最优解应选择消行', () => {
    const b = newBoard();
    // 底部两行几乎填满(各留 1 格洞在 x=9)
    for (let y = 18; y < 20; y += 1) {
      for (let x = 0; x < 9; x += 1) b[y * 10 + x] = PIECE_IDS.I;
    }
    const placements = solvePlacements(b, PIECE_IDS.I);
    // I 竖放(x=9, rot=1)一次补两层洞:存在消 2 行的候选,且它应是最高分
    expect(placements.filter((p) => p.cleared >= 2).length).toBeGreaterThan(0);
    expect(placements[0]?.cleared).toBe(2);
  });

  it('2-block lookahead:为下一块预留通道(单块贪心会制造死缝)', () => {
    // 场景:左侧一堆 9 高,右侧平地 3 高,当前 O 块、下一块 I。
    // 单块贪心倾向把 O 平放高处;2-block 会偏向右侧低位,给 I 留出消行通道。
    const b = newBoard();
    for (let y = 11; y < 20; y += 1) {
      for (let x = 0; x < 5; x += 1) b[y * 10 + x] = PIECE_IDS.O;
    }
    for (let y = 17; y < 20; y += 1) {
      for (let x = 5; x < 10; x += 1) b[y * 10 + x] = PIECE_IDS.O;
    }
    const greedy = solvePlacements(b, PIECE_IDS.O)[0];
    const look = solvePlacements2(b, PIECE_IDS.O, PIECE_IDS.I)[0];
    expect(greedy).toBeDefined();
    expect(look).toBeDefined();
    if (!greedy || !look) return;
    // 落定后:2-block 的盘面给 I 块的最优评分应不低于贪心的(排序核心性质)
    const evalForI = (p: { rot: number; x: number }): number => {
      const y = dropY(b, PIECE_IDS.O, p.rot, p.x);
      if (y < 0) return Number.NEGATIVE_INFINITY;
      const mid = b.slice();
      lockPiece(mid, PIECE_IDS.O, p.rot, p.x, y);
      const { board: after } = clearLines(mid);
      return solvePlacements(after, PIECE_IDS.I)[0]?.score ?? Number.NEGATIVE_INFINITY;
    };
    expect(evalForI(look)).toBeGreaterThanOrEqual(evalForI(greedy) - 1e-6);
  });

  it('7-bag:每 7 次发牌恰为一套完整七种', () => {
    const next = makeBag();
    const seen = new Set<number>();
    for (let i = 0; i < 7; i += 1) seen.add(next());
    expect(seen.size).toBe(7);
  });
});
