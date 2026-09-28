/**
 * 渲染器(docs/11 §7.3):关卡一次性绘入离屏 canvas(3.4MB),每帧只 blit
 * 视口 + 实体 + Laya 叠加层;canvas 背板 = 世界像素(高 240),CSS 铺满舞台、
 * `image-rendering: pixelated` 保持像素风。HUD/Laya 叠加画进 canvas——
 * 它们是游戏的一部分,不是 DOM 仪表盘。
 */

import type { GameState, Input } from './engine';
import type { Intent } from './planner';
import {
  COLS,
  GROUND_ROW,
  ROWS,
  TILE,
  T_BRICK,
  T_GROUND,
  T_PIPE,
  T_QBLOCK,
  T_STAIR,
  T_USED,
  type World,
} from './world1-1';

const SKY = '#5c94fc';

export function buildLevelCanvas(world: World): HTMLCanvasElement {
  const c = document.createElement('canvas');
  c.width = COLS * TILE;
  c.height = ROWS * TILE;
  const g = c.getContext('2d');
  if (!g) return c;
  g.fillStyle = SKY;
  g.fillRect(0, 0, c.width, c.height);
  // 云(少量装饰,静态)
  g.fillStyle = 'rgba(255,255,255,.85)';
  for (const [cx, cy] of [
    [40, 32],
    [90, 52],
    [520, 40],
    [1500, 36],
    [2400, 48],
    [3000, 40],
  ] as const) {
    g.fillRect(cx, cy, 48, 10);
    g.fillRect(cx + 8, cy - 6, 32, 8);
  }
  for (let row = 0; row < ROWS; row += 1) {
    for (let col = 0; col < COLS; col += 1) {
      const v = world.tiles[row * COLS + col] ?? 0;
      if (v === 0) continue;
      const x = col * TILE;
      const y = row * TILE;
      if (v === T_GROUND) {
        g.fillStyle = '#c0562a';
        g.fillRect(x, y, TILE, TILE);
        g.fillStyle = 'rgba(0,0,0,.18)';
        g.fillRect(x + TILE - 2, y, 2, TILE);
        if ((world.tiles[(row - 1) * COLS + col] ?? 0) === 0) {
          g.fillStyle = '#7ac74f';
          g.fillRect(x, y, TILE, 4);
        }
      } else if (v === T_BRICK) {
        g.fillStyle = '#a44e2b';
        g.fillRect(x, y, TILE, TILE);
        g.strokeStyle = 'rgba(0,0,0,.28)';
        g.lineWidth = 1;
        g.strokeRect(x + 0.5, y + 0.5, TILE - 1, TILE - 1);
        g.beginPath();
        g.moveTo(x, y + 8);
        g.lineTo(x + TILE, y + 8);
        g.stroke();
      } else if (v === T_QBLOCK) {
        g.fillStyle = '#e8a33d';
        g.fillRect(x, y, TILE, TILE);
        g.strokeStyle = '#9c5a12';
        g.lineWidth = 2;
        g.strokeRect(x + 1, y + 1, TILE - 2, TILE - 2);
        g.fillStyle = '#7c4a10';
        g.font = 'bold 10px monospace';
        g.fillText('?', x + 5, y + 11);
      } else if (v === T_USED) {
        g.fillStyle = '#9c6b3c';
        g.fillRect(x, y, TILE, TILE);
      } else if (v === T_PIPE) {
        const topPipe = (world.tiles[(row - 1) * COLS + col] ?? 0) !== T_PIPE;
        g.fillStyle = '#2fbf62';
        g.fillRect(x, y, TILE, TILE);
        g.fillStyle = col % 2 === 0 ? '#1d8f47' : '#26a854';
        g.fillRect(x, y, 4, TILE);
        if (topPipe) {
          g.fillStyle = '#0e7a3d';
          g.fillRect(x - 1, y, TILE + 2, 4);
        }
      } else if (v === T_STAIR) {
        g.fillStyle = '#c89a5b';
        g.fillRect(x, y, TILE, TILE);
        g.strokeStyle = 'rgba(0,0,0,.22)';
        g.strokeRect(x + 0.5, y + 0.5, TILE - 1, TILE - 1);
      }
    }
  }
  // 旗杆
  const fx = world.data.flagCol * TILE + 7;
  g.fillStyle = '#9aa7b5';
  g.fillRect(fx, 3 * TILE, 2, 10 * TILE);
  g.fillStyle = '#34d399';
  g.beginPath();
  g.moveTo(fx, 3 * TILE + 4);
  g.lineTo(fx - 12, 3 * TILE + 9);
  g.lineTo(fx, 3 * TILE + 14);
  g.fill();
  g.fillStyle = '#e7eaf0';
  g.beginPath();
  g.arc(fx + 1, 3 * TILE, 2, 0, Math.PI * 2);
  g.fill();
  return c;
}

export type FrameOverlays = {
  viewTiles: number;
  intent: Intent | null;
  heldInput: Input;
  /** 距上次决策的 tick 数(涟漪用);null = 无涟漪。 */
  rippleAge: number | null;
  /** HUD 画进 canvas(无头截图对照用);UI 走 DOM 覆盖层(docs/15 §1)。 */
  hudInCanvas?: boolean;
};

function chipText(input: Input): string {
  return (
    `${input.left ? '←' : ''}${input.right ? '→' : ''}${input.run ? '⇧' : ''}${input.jump ? '⤒' : ''}` ||
    '·'
  );
}

export function drawFrame(
  ctx: CanvasRenderingContext2D,
  level: HTMLCanvasElement,
  world: World,
  s: GameState,
  o: FrameOverlays,
): void {
  const viewW = o.viewTiles * TILE;
  const cam = Math.min(Math.max(0, Math.floor(s.camX)), COLS * TILE - viewW);
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(level, cam, 0, viewW, ROWS * TILE, 0, 0, viewW, ROWS * TILE);

  // 金币
  for (const c of s.coins) {
    if (c.taken) continue;
    const x = c.col * TILE + 5 - cam;
    if (x < -8 || x > viewW + 8) continue;
    ctx.fillStyle = '#fcd34d';
    ctx.beginPath();
    ctx.arc(c.col * TILE + 8 - cam, c.row * TILE + 8, 3.5, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = '#b57e14';
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  // Goomba
  for (const g of s.goombas) {
    if (!g.alive && g.squashT <= 0) continue;
    const x = g.x - cam;
    if (x < -20 || x > viewW + 20) continue;
    if (!g.alive) {
      ctx.fillStyle = '#8b5a2b';
      ctx.fillRect(x + 1, g.y + 10, 14, 6);
      continue;
    }
    ctx.fillStyle = '#8b5a2b';
    ctx.fillRect(x + 1, g.y + 2, 14, 14);
    ctx.fillStyle = '#f4efe6';
    ctx.fillRect(x + 3, g.y + 4, 4, 4);
    ctx.fillRect(x + 9, g.y + 4, 4, 4);
    ctx.fillStyle = '#1b1b1b';
    ctx.fillRect(x + 4, g.y + 5, 2, 2);
    ctx.fillRect(x + 10, g.y + 5, 2, 2);
  }

  // 马里奥
  const m = s.mario;
  {
    const x = m.x - cam;
    ctx.fillStyle = '#e23b3b';
    ctx.fillRect(x + 2, m.y + 2, 12, 14);
    ctx.fillStyle = '#7c1d1d';
    ctx.fillRect(x + (m.face === 1 ? 2 : 8), m.y + 2, 6, 4);
    ctx.fillStyle = '#5b3a1e';
    ctx.fillRect(x + 3, m.y + 16, 4, 0);
    ctx.fillStyle = '#2b5cb8';
    ctx.fillRect(x + 4, m.y + 10, 8, 4);
  }

  // ---- Laya 叠加层 ----
  // 意图目标(品红虚线,与 Tetris 目标框同语义)
  if (o.intent?.hazard) {
    const h = o.intent.hazard;
    ctx.strokeStyle = 'rgba(232,121,249,.9)';
    ctx.setLineDash([2, 2]);
    ctx.lineWidth = 1;
    ctx.strokeRect(h.startX - cam, (GROUND_ROW - 4) * TILE, h.endX - h.startX, 4 * TILE);
    ctx.setLineDash([]);
  }
  // 决策涟漪(一次性脉冲)
  if (o.rippleAge !== null && o.rippleAge < 18) {
    const r = 4 + o.rippleAge * 1.4;
    ctx.strokeStyle = `rgba(52,211,153,${(18 - o.rippleAge) / 18})`;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.arc(m.x + 8 - cam, m.y + 8, r, 0, Math.PI * 2);
    ctx.stroke();
  }
  // 输入量子芯片(右上角)
  {
    const label = chipText(o.heldInput);
    ctx.fillStyle = 'rgba(11,11,16,.78)';
    ctx.fillRect(viewW - 58, 4, 54, 11);
    ctx.strokeStyle = 'rgba(42,48,64,.9)';
    ctx.strokeRect(viewW - 57.5, 4.5, 53, 10);
    ctx.fillStyle = '#34d399';
    ctx.font = '8px monospace';
    ctx.fillText(`IN ${label}`, viewW - 52, 12);
  }

  // ---- HUD(默认走 DOM 覆盖层;hudInCanvas 供无头截图/回归对照) ----
  if (o.hudInCanvas === true) {
    ctx.font = 'bold 8px monospace';
    const shadow = (text: string, x: number, y: number): void => {
      ctx.fillStyle = 'rgba(0,0,0,.55)';
      ctx.fillText(text, x + 1, y + 1);
      ctx.fillStyle = '#ffffff';
      ctx.fillText(text, x, y);
    };
    shadow(`SCORE ${String(s.score).padStart(6, '0')}`, 8, 12);
    shadow(`COINS x${String(s.coinCount).padStart(2, '0')}`, 70, 12);
    shadow('WORLD 1-1', 128, 12);
    shadow(`TIME ${String(Math.max(0, s.timeUnits)).padStart(3, '0')}`, viewW - 44, 12);
  }
  void world;
}
