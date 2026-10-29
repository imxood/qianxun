/**
 * System 2 规划器(docs/11 §5 + docs/16 §2)。
 *
 * v2 核心变化:**从"向前扫描第一个危险"的启发式,换成 rollout 搜索**。
 * - 做法是标准的「rollout 策略 + 单步策略改进」:给一个无递归的基线策略
 *   (always run right + 遇险起跳),对每个候选动作先执行 commitTicks,
 *   再交给基线跑满 horizon,用终局 surrogate 给分;argmax 即规划动作。
 * - 依据:对游戏类状态-动作选择,启发式+搜索**结构上优于**一次性分类器
 *   (Jev 俄罗斯方块 62.8%、ViZDoom Basic 56/128;TypeSafe 自己承认
 *   "非 AI 的 Doom bot 会打得更好")。System 1(Laya)只在搜索给出的
 *   候选集里挑,置信度低则回落到搜索——即 PlayJev 的"反射+置信度移交搜索"。
 * - 引擎是确定性纯函数,因此搜索结果可复现、可单测;代价是每拍 O(候选×horizon)
 *   次 step,horizon/候选数都是 policy 沙盒可调字段。
 */

import { step, type GameState, type Input } from './engine';
import { getPolicy, type PolicyProfile } from './policy';
import { COLS, JUMP, ROWS, TILE, WORLD_1_1, isSolid, type World } from './world1-1';

export type Action =
  | 'left'
  | 'right'
  | 'run_right'
  | 'run_left'
  | 'jump'
  | 'jump_right'
  | 'jump_left'
  | 'jump_run_right'
  | 'jump_run_left'
  | 'fire_right'
  | 'idle';

export const ACTIONS: Action[] = [
  'idle',
  'left',
  'right',
  'run_left',
  'run_right',
  'jump',
  'jump_left',
  'jump_right',
  'jump_run_right',
  'jump_run_left',
  'fire_right',
];

export const ACTION_HINTS: Record<Action, string> = {
  idle: 'Stand still; use when waiting or nothing useful can be done',
  left: 'Walk one step left',
  right: 'Walk right toward the goal',
  run_left: 'Run left',
  run_right: 'Run right toward the goal; default when the way is clear',
  jump: 'Jump straight up; use for blocks overhead',
  jump_left: 'Jump moving left',
  jump_right: 'Short jump moving right; gaps within walking jump reach',
  jump_run_left: 'Running jump left',
  jump_run_right: 'Running jump right; required for wide gaps and 4-high pipes',
  fire_right: 'Walk right while throwing a fireball; only when powered up',
};

export function actionInput(a: Action): Input {
  const right = a.endsWith('right');
  const left = a.endsWith('left');
  return {
    left,
    right,
    run: a.startsWith('jump_run') || a === 'run_right' || a === 'run_left',
    jump: a.startsWith('jump'),
    fire: a === 'fire_right',
  };
}

// ---------------------------------------------------------------- 危险

export type HazardKind = 'gap' | 'pipe' | 'goomba' | 'koopa' | 'stair' | 'plant';
export type Hazard = {
  kind: HazardKind;
  startX: number;
  endX: number;
  label: string;
  ref?: unknown;
};

function tileSolidRow(s: GameState, col: number): number {
  for (let r = 0; r < ROWS; r += 1) if (isSolid(s.tiles[r * COLS + col] ?? 0)) return r;
  return ROWS;
}

/** 前方 lookPx 内的危险列表(按 startPx 升序)。 */
export function nextHazards(s: GameState, world: World, lookPx = 176): Hazard[] {
  const front = s.mario.x + 14;
  const out: Hazard[] = [];
  for (const g of world.gaps) {
    const startX = g.startCol * TILE;
    const endX = (g.endCol + 1) * TILE;
    if (endX > front - 8 && startX < front + lookPx)
      out.push({ kind: 'gap', startX, endX, label: `gap w=${g.endCol - g.startCol + 1}` });
  }
  for (const p of world.data.pipes) {
    const startX = p.col * TILE;
    const endX = (p.col + 2) * TILE;
    if (endX > front - 8 && startX < front + lookPx)
      out.push({ kind: 'pipe', startX, endX, label: `pipe h=${p.height}`, ref: p });
  }
  for (const g of s.goombas) {
    if (!g.alive) continue;
    if (g.x > front - 4 && g.x < front + lookPx)
      out.push({ kind: 'goomba', startX: g.x, endX: g.x + 16, label: 'goomba', ref: g });
  }
  for (const k of s.koopas) {
    if (!k.alive) continue;
    if (k.x > front - 4 && k.x < front + lookPx)
      out.push({
        kind: 'koopa',
        startX: k.x,
        endX: k.x + 16,
        label: k.mode === 'walk' ? 'koopa' : k.mode === 'shell' ? 'shell' : 'slide',
        ref: k,
      });
  }
  for (const p of s.plants) {
    if (p.dead || p.out <= 0.25) continue;
    const startX = p.col * TILE;
    if (startX > front - 8 && startX < front + lookPx)
      out.push({
        kind: 'plant',
        startX,
        endX: startX + 2 * TILE,
        label: `plant r${p.topRow}`,
        ref: p,
      });
  }
  // 台阶墙:相邻两列台阶高度递增处(ascending step = 一面台阶墙,含 1 瓦台阶)
  const heights = new Map<number, number>();
  for (const st of world.data.stairs) heights.set(st.col, st.height);
  for (const st of world.data.stairs) {
    const prev = heights.get(st.col - 1) ?? 0;
    if (st.height > prev) {
      const startX = st.col * TILE;
      if (startX > front - 8 && startX < front + lookPx)
        out.push({
          kind: 'stair',
          startX,
          endX: startX + TILE,
          label: `step h=${st.height - prev}`,
        });
    }
  }
  out.sort((a, b) => a.startX - b.startX);
  return out;
}

// ---------------------------------------------------------------- 搜索

/** 深拷贝(仿真是纯函数,必须与原局完全隔离)。 */
export function cloneState(s: GameState): GameState {
  return {
    ...s,
    mario: { ...s.mario },
    goombas: s.goombas.map((g) => ({ ...g })),
    koopas: s.koopas.map((k) => ({ ...k })),
    plants: s.plants.map((p) => ({ ...p })),
    items: s.items.map((i) => ({ ...i })),
    fireballs: s.fireballs.map((f) => ({ ...f })),
    coins: s.coins.map((c) => ({ ...c })),
    blocks: new Map([...s.blocks].map(([k, v]) => [k, { ...v }] as const)),
    tiles: s.tiles.slice(),
    deaths: s.deaths.map((d) => ({ ...d })),
    win: { ...s.win },
  };
}

/**
 * 基线 rollout 策略(无递归、可单测):
 * 一律向右助跑;脚下前方 1.6 瓦内是坑 / 有墙 / 有敌人时起跳。
 * 它本身很"笨"(不理解食人花/龟壳),正是搜索要改进的基线。
 */
export function rolloutInput(s: GameState, world: World): Input {
  if (s.phase !== 'running') return IDLE;
  const m = s.mario;
  const col = Math.floor((m.x + 16 + 8) / TILE);
  const aheadRow = tileSolidRow(s, col);
  const footRow = Math.floor((m.y + 16 - 1) / TILE);
  // 脚下前方是坑 → 跳
  let jump = false;
  if (m.onGround && aheadRow === ROWS) jump = true;
  // 前方是墙(实心行高于脚下行)→ 跳
  if (m.onGround && aheadRow < footRow) jump = true;
  // 前方有敌人 → 跳
  for (const g of s.goombas) {
    if (g.alive && g.active && g.x - m.x > 0 && g.x - m.x < 26) jump = true;
  }
  for (const k of s.koopas) {
    if (k.alive && k.active && k.mode === 'walk' && k.x - m.x > 0 && k.x - m.x < 26) jump = true;
  }
  for (const p of s.plants) {
    if (!p.dead && p.out > 0.4 && p.col * TILE - m.x > 0 && p.col * TILE - m.x < 40) jump = true;
  }
  void world;
  return { left: false, right: true, run: true, jump: m.vy < 0 || jump, fire: false };
}

const IDLE: Input = { left: false, right: false, run: false, jump: false, fire: false };

/**
 * 终局 surrogate:越大越好。死亡是灾难(覆盖一切),其余按
 * 进度 / 分数 / 存活时间 / 敌人清理 加权,并对"贴脸危险"做势能塑形。
 */
export function evaluateState(s0: GameState, s1: GameState, world: World): number {
  if (s1.phase === 'dead') return -1_000_000;
  if (s1.phase === 'won') return 1_000_000;
  let v = 1000 * (s1.mario.x - s0.mario.x);
  v += 20 * (s1.score - s0.score);
  v -= 8 * (s0.timeUnits - s1.timeUnits);
  const alive0 = s0.goombas.filter((g) => g.alive).length + s0.koopas.filter((k) => k.alive).length;
  const alive1 = s1.goombas.filter((g) => g.alive).length + s1.koopas.filter((k) => k.alive).length;
  v += 120 * (alive0 - alive1);
  // 势能塑形:最近危险越近扣分越重(鼓励提前决策而不是贴脸反应)
  const hz = nextHazards(s1, world, 96)[0];
  if (hz && hz.kind !== 'plant') {
    const d = hz.startX - (s1.mario.x + 14);
    if (d >= 0 && d < 40) v -= (40 - d) * 6;
  }
  // 贴地奖励(避免浮空乱飘)
  if (s1.mario.onGround) v += 6;
  return v;
}

export type Scored = { action: Action; score: number };

/**
 * 对候选动作做 rollout 评分(确定性)。
 * @param commitTicks 候选动作被"按住"的 tick 数,之后交给基线策略。
 * @param horizon    总仿真 tick 数。
 */
export function scoreAction(
  s: GameState,
  world: World,
  action: Action,
  commitTicks: number,
  horizon: number,
): number {
  const sim = cloneState(s);
  const held = actionInput(action);
  for (let t = 0; t < horizon; t += 1) {
    const input = t < commitTicks ? held : rolloutInput(sim, world);
    step(world, sim, input);
    if (sim.phase !== 'running') break;
  }
  return evaluateState(s, sim, world);
}

// ---------------------------------------------------------------- 意图

export type IntentType =
  'clear' | 'jump_gap' | 'hop_pipe' | 'engage' | 'climb' | 'evade' | 'powered';
export type Intent = {
  type: IntentType;
  hazard: Hazard | null;
  /** 规划认为朝目标合法且有益的动作(约束解码用,2~3 个)。 */
  candidates: Action[];
  note: string;
  /** 搜索得分(与 candidates 平行;UI 可直接画条形)。 */
  scores?: number[];
  /** 搜索出的最佳动作(autopilot 直接执行它)。 */
  best?: Action;
};

export function defaultCommit(horizon: number): number {
  return Math.max(4, Math.round(horizon * 0.35));
}

/**
 * System 2:搜索出下一意图。
 * 候选集 = 搜索得分 Top-K(K 默认 3,保持 Laya 的选项数在"不会崩"的范围内——
 * Laya 在 >20 选项时急剧退化,Banking77 77 类只有 0.425)。
 *
 * 结果按 (state, tick, 搜索参数) 缓存:autopilot 每 tick 调一次、叠加层还要
 * 再调一次,没有缓存就是每帧一次完整 rollout,主循环直接被打爆。
 */
const intentCache = new WeakMap<GameState, { tick: number; key: string; intent: Intent }>();

function searchKey(P: PolicyProfile): string {
  return `${P.searchHorizon}|${P.searchTopK}|${P.searchLookPx}`;
}

function computeIntent(s: GameState, world: World, P: PolicyProfile): Intent {
  const hazards = nextHazards(s, world, P.searchLookPx);
  const h = hazards[0] ?? null;
  const horizon = P.searchHorizon;
  const commit = defaultCommit(horizon);

  // 形态受限:只有 fire 形态才把 fire_right 放进搜索池
  const pool: Action[] =
    s.mario.power === 'fire' ? ACTIONS : ACTIONS.filter((a) => a !== 'fire_right');

  const scored: Scored[] = pool.map((a) => ({
    action: a,
    score: scoreAction(s, world, a, commit, horizon),
  }));
  scored.sort((a, b) => b.score - a.score || ACTIONS.indexOf(a.action) - ACTIONS.indexOf(b.action));
  const top = scored.slice(0, P.searchTopK);
  const best = top[0]?.action ?? 'run_right';

  let type: IntentType = 'clear';
  if (s.mario.power !== 'small') type = 'powered';
  if (h) {
    if (h.kind === 'gap') type = 'jump_gap';
    else if (h.kind === 'pipe') type = 'hop_pipe';
    else if (h.kind === 'stair') type = 'climb';
    else if (h.kind === 'plant') type = 'evade';
    else type = 'engage';
  }
  const note = h ? `${h.label} dx=${Math.round((h.startX - (s.mario.x + 14)) / TILE)}t` : 'clear';
  return {
    type,
    hazard: h,
    candidates: top.map((t) => t.action),
    note,
    scores: top.map((t) => Math.round(t.score)),
    best,
  };
}

export function nextIntent(s: GameState, world: World = WORLD_1_1, policy?: PolicyProfile): Intent {
  const P = policy ?? getPolicy();
  const key = searchKey(P);
  const c = intentCache.get(s);
  if (c && c.tick === s.tick && c.key === key) return c.intent;
  const intent = computeIntent(s, world, P);
  intentCache.set(s, { tick: s.tick, key, intent });
  return intent;
}

// ---------------------------------------------------------------- autopilot

/**
 * autopilot = System 2 直接接管(不再有触发窗启发式)。
 * 搜索保证演示必达:它优化的目标函数里死亡是 -1e6,任何"能活着往前"的
 * 动作都会压过冒险动作。
 */
export function autopilotInput(
  s: GameState,
  world: World = WORLD_1_1,
  policy?: PolicyProfile,
): Input {
  if (s.phase !== 'running') return IDLE;
  const intent = nextIntent(s, world, policy);
  return actionInput(intent.best ?? intent.candidates[0] ?? 'run_right');
}

/** 跳跃可达性自检(单测用):给定宽度,跑跳是否够。 */
export function canClearGap(widthTiles: number): boolean {
  return widthTiles <= JUMP.runDistTiles;
}
