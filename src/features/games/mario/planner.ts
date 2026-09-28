/**
 * System 2 规划器(docs/11 §5):从关卡数据推导危险场,给出
 * 「下一个意图 + 候选动作约束」;autopilot = 规划器直接生成输入(安全机动)。
 *
 * - 跳跃可达性全部引用 world1-1 的 JUMP 表(同一物理常量,单测锁定)。
 * - 关卡静态 + 相机只进不退 ⇒ 感知就是"向前扫描第一个危险",不需要图搜索。
 * - nextIntent 供 reflex 模式写 subgoal 与候选集;autopilotInput 供
 *   自驾模式与 ESCALATE 兜底直接生成输入。
 */

import type { GameState, Input } from './engine';
import { getPolicy, type PolicyProfile } from './policy';
import { JUMP, TILE, WORLD_1_1, type World } from './world1-1';

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
  'jump_run_left',
  'jump_run_right',
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
};

export function actionInput(a: Action): Input {
  const right = a.endsWith('right') || a === 'right' || a === 'run_right';
  const left = a.endsWith('left') || a === 'left' || a === 'run_left';
  return {
    left,
    right,
    run: a.startsWith('jump_run') || a === 'run_right' || a === 'run_left',
    jump: a.startsWith('jump'),
  };
}

export type HazardKind = 'gap' | 'pipe' | 'goomba' | 'stair';
export type Hazard = {
  kind: HazardKind;
  startX: number;
  endX: number;
  label: string;
  ref?: unknown;
};

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

export type IntentType = 'clear' | 'jump_gap' | 'hop_pipe' | 'engage' | 'climb';
export type Intent = {
  type: IntentType;
  hazard: Hazard | null;
  /** 规划认为朝目标合法且有益的动作(约束解码用,2~3 个)。 */
  candidates: Action[];
  note: string;
};

export function nextIntent(s: GameState, world: World = WORLD_1_1): Intent {
  const hazards = nextHazards(s, world);
  const h = hazards[0];
  if (!h)
    return {
      type: 'clear',
      hazard: null,
      candidates: ['run_right', 'jump_right', 'right'],
      note: 'clear',
    };
  const dx = h.startX - (s.mario.x + 14);

  if (h.kind === 'gap') {
    const w = (h.endX - h.startX) / TILE;
    const needRun = w > JUMP.walkDistTiles - 1;
    const commit = dx <= TILE * 3.2;
    return {
      type: 'jump_gap',
      hazard: h,
      candidates: commit
        ? needRun
          ? ['jump_run_right', 'jump_right']
          : ['jump_right', 'jump_run_right']
        : ['run_right', 'jump_run_right'],
      note: `${h.label} dx=${Math.round(dx / TILE)}t`,
    };
  }
  if (h.kind === 'pipe') {
    const commit = dx <= 24;
    return {
      type: 'hop_pipe',
      hazard: h,
      candidates: commit ? ['jump_right', 'jump_run_right'] : ['run_right', 'jump_right'],
      note: `${h.label} dx=${Math.round(dx)}px`,
    };
  }
  if (h.kind === 'goomba') {
    const commit = dx <= 40;
    return {
      type: 'engage',
      hazard: h,
      candidates: commit ? ['jump_right', 'run_right', 'idle'] : ['run_right', 'right', 'idle'],
      note: `${h.label} dx=${Math.round(dx)}px`,
    };
  }
  const commit = dx <= 20;
  return {
    type: 'climb',
    hazard: h,
    candidates: commit ? ['jump_right', 'jump_run_right'] : ['run_right', 'jump_right'],
    note: `${h.label} dx=${Math.round(dx)}px`,
  };
}

/**
 * autopilot 安全机动:规划器直接生成输入。
 *
 * 三条纪律(实测教训,见 engine.test 的通关用例):
 * 1. **跳跃进窗触发**:只在接近窗口内 jump,其余 tick 松键——jumpLatch 只有在
 *    input.jump=false 时才清零,持续按住会导致落地后永远跳不起来(卡死管道);
 * 2. **空中保持满弧**(jump=true):hold 语义下空中松键会截断跳高;
 * 3. **贴墙后撤重助跑**:h4 管道必须满速助跑才跳得过(跳高 69px vs 管 64px,
 *    余量 5px),贴墙零速度起跳过不去 → 后撤 ~26 tick 重新进窗。
 * 后撤记忆用 WeakMap 挂在 state 上:同一 state 的调用序列确定 ⇒ 行为确定。
 * 计数按 **tick 差**(s.tick - 起始)而非调用次数——自驾每 tick 调用、反射
 * 每拍(250ms)调用,节奏不同,按次数计会把 26 tick 的后撤拖成 6 秒。
 * policy 可注入局内快照(docs/13 不变量①);不传则读全局热更新值。
 */
const backing = new WeakMap<GameState, number>();

export function autopilotInput(
  s: GameState,
  world: World = WORLD_1_1,
  policy?: PolicyProfile,
): Input {
  if (s.phase !== 'running') return { left: false, right: false, run: false, jump: false };
  const front = s.mario.x + 14;
  const h = nextHazards(s, world, 80)[0];
  const dist = h ? h.startX - front : Number.POSITIVE_INFINITY;

  const P = policy ?? getPolicy();
  const backingStart = backing.get(s);
  if (backingStart !== undefined) {
    if (s.tick - backingStart < P.backoffTicks) {
      return { left: true, right: false, run: true, jump: false };
    }
    backing.delete(s);
  }
  // 贴墙检测:有速度意图却推不动(vx≈0)且危险近在咫尺
  if (s.mario.onGround && s.mario.vx < 0.05 && h !== undefined && dist <= 3) {
    backing.set(s, s.tick);
    return { left: true, right: false, run: true, jump: false };
  }

  let trigger = false;
  if (h) {
    // 触发窗来自策略沙箱(本地 agent 可调,见 policy.ts);gap 的下界
    // 救援窗语义:踩跳落点悬在坑缘时仍允许起跳。
    if (h.kind === 'gap') trigger = dist <= P.gapWindow[1] && dist > P.gapWindow[0];
    else if (h.kind === 'pipe') trigger = dist <= P.pipeWindow[1] && dist >= P.pipeWindow[0];
    else if (h.kind === 'goomba') trigger = dist <= P.goombaWindow[1] && dist >= P.goombaWindow[0];
    else trigger = dist <= P.stairWindow[1] && dist > P.stairWindow[0];
  }
  // 空中只在上升段保持 jump:截断重力只作用于上升,而下降段松键让
  // jumpLatch 及时清零——否则落地时 latch 卡死,贴窗触发全失效(实测教训)。
  return { left: false, right: true, run: true, jump: s.mario.vy < 0 || trigger };
}
