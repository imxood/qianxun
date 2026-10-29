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
  drawCastle(g, world);
  return c;
}

/** 城堡:砖墙 + 门洞 + 塔楼(通关终点,纯装饰)。 */
function drawCastle(g: CanvasRenderingContext2D, world: World): void {
  const x = world.data.castleCol * TILE;
  const baseY = GROUND_ROW * TILE;
  g.fillStyle = '#8a6a4a';
  g.fillRect(x, baseY - 5 * TILE, 5 * TILE, 5 * TILE);
  g.fillStyle = '#a07f5c';
  g.fillRect(x + 2, baseY - 3 * TILE, 1.5 * TILE, 3 * TILE);
  // 塔楼齿
  g.fillStyle = '#8a6a4a';
  for (let i = 0; i < 5; i += 1) g.fillRect(x + i * TILE, baseY - 6 * TILE, TILE, TILE);
  // 门洞
  g.fillStyle = '#2a1a10';
  g.fillRect(x + 1.5 * TILE, baseY - 2 * TILE, 2 * TILE, 2 * TILE);
  // 窗
  g.fillStyle = '#2a1a10';
  g.fillRect(x + 0.5 * TILE, baseY - 4 * TILE, TILE, TILE);
  g.fillRect(x + 3.5 * TILE, baseY - 4 * TILE, TILE, TILE);
}

export type FrameOverlays = {
  viewTiles: number;
  intent: Intent | null;
  heldInput: Input;
  /** 距上次决策的 tick 数(涟漪用);null = 无涟漪。 */
  rippleAge: number | null;
  /** HUD 画进 canvas(无头截图对照用);UI 走 DOM 覆盖层(docs/15 §1)。 */
  hudInCanvas?: boolean;
  /** 画 System 2 搜索得分条(默认开,可在 UI 关)。 */
  showSearch?: boolean;
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
  const m = s.mario;
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(level, cam, 0, viewW, ROWS * TILE, 0, 0, viewW, ROWS * TILE);

  // 旗杆(动态:通关时随下滑的马里奥下降)
  {
    const fx = world.data.flagCol * TILE + 7;
    ctx.fillStyle = '#9aa7b5';
    ctx.fillRect(fx, 3 * TILE, 2, 10 * TILE);
    ctx.fillStyle = '#e7eaf0';
    ctx.beginPath();
    ctx.arc(fx + 1, 3 * TILE, 2, 0, Math.PI * 2);
    ctx.fill();
    const flagY = s.win.stage === 'slide' || s.win.stage === 'walk' ? m.y + 4 : 3 * TILE + 4;
    ctx.fillStyle = '#34d399';
    ctx.beginPath();
    ctx.moveTo(fx, flagY);
    ctx.lineTo(fx - 12, flagY + 5);
    ctx.lineTo(fx, flagY + 10);
    ctx.fill();
  }

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

  // 乌龟(行走 / 静止壳 / 滑行壳)
  for (const k of s.koopas) {
    if (!k.alive) continue;
    const x = k.x - cam;
    if (x < -20 || x > viewW + 20) continue;
    if (k.mode === 'walk') {
      ctx.fillStyle = '#3fbf4f';
      ctx.fillRect(x + 2, k.y + 1, 12, 9); // 头/身
      ctx.fillRect(x + (k.vx < 0 ? 0 : 11), k.y + 3, 5, 6); // 朝向
      ctx.fillStyle = '#e8a33d';
      ctx.fillRect(x + 3, k.y + 8, 10, 8); // 壳
      ctx.fillStyle = '#a86f1c';
      ctx.fillRect(x + 5, k.y + 10, 6, 4);
      ctx.fillStyle = '#1b1b1b';
      ctx.fillRect(x + (k.vx < 0 ? 1 : 13), k.y + 5, 2, 2);
    } else {
      // 壳(滑行时描白边表示"危险")
      ctx.fillStyle = k.mode === 'slide' ? '#f8fafc' : '#e8a33d';
      ctx.fillRect(x + 1, k.y + 2, 14, 12);
      ctx.fillStyle = k.mode === 'slide' ? '#cbd5e1' : '#a86f1c';
      ctx.fillRect(x + 4, k.y + 5, 8, 6);
    }
  }

  // 食人花
  for (const p of s.plants) {
    if (p.dead || p.out <= 0) continue;
    const h = 2 * TILE * p.out;
    const x = p.col * TILE + 2 - cam;
    const y = p.topRow * TILE - h;
    if (x > viewW + 20 || x + 2 * TILE < -20) continue;
    ctx.fillStyle = '#2fbf62';
    ctx.fillRect(x + 4, y + 6, 2 * TILE - 12, h - 6); // 茎
    ctx.fillStyle = '#e23b3b';
    ctx.fillRect(x + 2, y, 2 * TILE - 8, 9); // 头
    ctx.fillStyle = '#f4efe6';
    ctx.fillRect(x + 5, y + 3, 3, 3);
    ctx.fillRect(x + 2 * TILE - 14, y + 3, 3, 3);
  }

  // 道具(蘑菇 / 火焰花 / 吐出的金币)
  for (const it of s.items) {
    if (it.gone) continue;
    const x = it.x - cam;
    if (x < -20 || x > viewW + 20) continue;
    if (it.kind === 'coin') {
      ctx.fillStyle = '#fcd34d';
      ctx.beginPath();
      ctx.arc(x + 8, it.y + 8, 4, 0, Math.PI * 2);
      ctx.fill();
    } else if (it.kind === 'mushroom') {
      ctx.fillStyle = '#e23b3b';
      ctx.fillRect(x + 1, it.y + 2, 14, 8); // 菌盖
      ctx.fillStyle = '#f4efe6';
      ctx.fillRect(x + 4, it.y + 4, 3, 3);
      ctx.fillRect(x + 9, it.y + 5, 3, 3);
      ctx.fillStyle = '#f0d9b5';
      ctx.fillRect(x + 4, it.y + 10, 8, 6); // 柄
    } else {
      // 火焰花
      ctx.fillStyle = '#2fbf62';
      ctx.fillRect(x + 7, it.y + 10, 2, 6);
      ctx.fillStyle = '#f97316';
      ctx.fillRect(x + 3, it.y + 3, 10, 8);
      ctx.fillStyle = '#fde047';
      ctx.fillRect(x + 5, it.y + 5, 3, 3);
      ctx.fillRect(x + 9, it.y + 5, 3, 3);
    }
  }

  // 火球
  for (const f of s.fireballs) {
    const x = f.x - cam;
    if (x < -12 || x > viewW + 12) continue;
    ctx.fillStyle = '#f97316';
    ctx.beginPath();
    ctx.arc(x + 4, f.y + 4, 4, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = '#fde047';
    ctx.beginPath();
    ctx.arc(x + 4, f.y + 4, 2, 0, Math.PI * 2);
    ctx.fill();
  }

  // 马里奥(形态决定体型与配色;无敌期闪烁)
  if (!m.hidden && !(m.invuln > 0 && Math.floor(s.tick / 4) % 2 === 1)) {
    const x = m.x - cam;
    const big = m.power !== 'small';
    const h = big ? 26 : 14;
    const top = m.y + 16 - h;
    const shirt = m.power === 'fire' ? '#f8fafc' : '#e23b3b';
    const pants = m.power === 'fire' ? '#e23b3b' : '#2b5cb8';
    ctx.fillStyle = shirt;
    ctx.fillRect(x + 2, top + (big ? 2 : 0), 12, big ? 12 : 8);
    ctx.fillStyle = '#f0d9b5'; // 脸
    ctx.fillRect(x + 4, top + (big ? 6 : 3), 8, big ? 8 : 5);
    ctx.fillStyle = '#5b3a1e'; // 头发/胡子
    ctx.fillRect(x + (m.face === 1 ? 4 : 9), top + (big ? 4 : 1), 5, big ? 4 : 3);
    ctx.fillStyle = '#1b1b1b';
    ctx.fillRect(x + (m.face === 1 ? 9 : 5), top + (big ? 8 : 5), 2, 2);
    ctx.fillStyle = pants;
    ctx.fillRect(x + 2, m.y + 9, 12, big ? 5 : 3);
    ctx.fillStyle = '#3a2412'; // 鞋
    ctx.fillRect(x + 1, m.y + 13, 6, 3);
    ctx.fillRect(x + 9, m.y + 13, 6, 3);
    if (m.power === 'fire') {
      // 白帽红帽檐
      ctx.fillStyle = '#f8fafc';
      ctx.fillRect(x + 2, top, 12, 4);
      ctx.fillStyle = '#e23b3b';
      ctx.fillRect(x + 2, top + 3, 12, 2);
    } else if (big) {
      ctx.fillStyle = '#e23b3b';
      ctx.fillRect(x + 2, top, 12, 4); // 大马里奥红帽
    }
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
  // System 2 搜索得分条(左上):Top-K 候选动作的 rollout 终局分
  if (o.showSearch && o.intent?.scores && o.intent.candidates.length > 0) {
    const scores = o.intent.scores;
    const lo = Math.min(...scores);
    const hi = Math.max(...scores);
    const span = Math.max(1, hi - lo);
    ctx.font = 'bold 7px monospace';
    o.intent.candidates.forEach((a, i) => {
      const y = 20 + i * 9;
      const w = 70 * ((scores[i]! - lo) / span);
      ctx.fillStyle = 'rgba(11,11,16,.72)';
      ctx.fillRect(6, y - 6, 96, 8);
      ctx.fillStyle = i === 0 ? '#34d399' : 'rgba(52,211,153,.45)';
      ctx.fillRect(7, y - 5, w, 6);
      ctx.fillStyle = i === 0 ? '#ffffff' : 'rgba(230,238,248,.8)';
      ctx.fillText(a, 7, y);
    });
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
    shadow(`WORLD ${world.data.name}`, 128, 12);
    const pw = s.mario.power === 'fire' ? 'FIRE' : s.mario.power === 'big' ? 'SUPER' : '';
    if (pw) shadow(pw, 190, 12);
    shadow(`TIME ${String(Math.max(0, s.timeUnits)).padStart(3, '0')}`, viewW - 44, 12);
  }
}
