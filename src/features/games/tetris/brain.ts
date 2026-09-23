/**
 * Laya 反射决策端(System 1):把棋局快照变成 laya-server 的
 * {state, questions} 调用,再做置信度门控三分支——
 *   conf ≥ 0.85      EXECUTE   直接执行反射动作
 *   0.5 ≤ conf <0.85 RE-SENSE  再感知一轮(最多 2 轮,防抖动)
 *   conf < 0.5       ESCALATE  升级 System 2 求解器直接放置
 *
 * 三种驱动模式:
 *   reflex — System 2 给出目标放置,写入 state;Laya 逐步选动作逼近它
 *   pure   — 不给目标,纯 System 1 看盘选动作(检验反射层真实水平)
 *   solver — 不调 Laya,System 2 直接放置(对照基线)
 */

import {
  COLS,
  ROWS,
  solvePlacements2,
  pieceName,
  features,
  stackHeight,
  type Board,
  type Placement,
} from './engine';

export type DriveMode = 'reflex' | 'pure' | 'solver' | 'human';

export type Gate = 'EXECUTE' | 'RE_SENSE' | 'ESCALATE';

export const ACTIONS = [
  'shift_left',
  'shift_right',
  'rotate',
  'rotate_ccw',
  'soft_drop',
  'hard_drop',
  'wait',
] as const;
export type Action = (typeof ACTIONS)[number];

export type Snapshot = {
  board: Board;
  pieceId: number;
  pieceRot: number;
  pieceX: number;
  pieceY: number;
  nextPieceId: number;
};

export type Decision = {
  action: Action;
  conf: number;
  gate: Gate;
  latencyMs: number;
  probs: Partial<Record<Action, number>>;
  danger: number;
  /** ESCALATE 时 System 2 给出的目标放置。 */
  plan: Placement | null;
  /** RE_SENSE 后实际采用的是第几次感知。 */
  sensed: number;
};

const ENDPOINT_DEFAULT = 'http://127.0.0.1:10230';

/** 置信度阈值(与 JEV Lab 同款三分支)。
 * 注意:工单域 Laya conf 普遍 0.9+,而游戏棋盘文本是 OOD 域(实测空板
 * T 块 conf≈0.169),默认阈值按实测校准,并暴露到 UI 实时可调。 */
export const GATE_EXECUTE = 0.16;
export const GATE_ESCALATE = 0.12;
/** 连续 RE_SENSE 上限:第二次无论高低都执行,避免在边缘置信度上死循环。 */
const MAX_RE_SENSE = 2;

export type PlanTarget = { placement: Placement; pieceName: string };

/** System 2 规划:2-block lookahead(看一眼下一块),棋盘已满时为 null。 */
export function planTarget(snap: Snapshot): PlanTarget | null {
  const best = solvePlacements2(snap.board, snap.pieceId, snap.nextPieceId)[0];
  if (!best) return null;
  return {
    placement: best,
    pieceName: pieceName(snap.pieceId),
  };
}

/** 动作语义表(reflex 下动态 criteria 复用)。 */
export const ACTION_HINTS: Record<Action, string> = {
  shift_left: 'Move the falling piece one column left; use when target column is to the left',
  shift_right: 'Move the falling piece one column right; use when target column is to the right',
  rotate: 'Rotate clockwise; use when target rotation is ahead of current rotation',
  rotate_ccw:
    'Rotate counter-clockwise; use when a single counter-clockwise step reaches target rotation',
  soft_drop: 'Move the piece down one row; use when close to the target row',
  hard_drop: 'Lock instantly; use only when current position already matches the target',
  wait: 'Do nothing this tick; use when nothing useful can be done',
};

/**
 * 约束解码:reflex 模式下只给「朝目标合法的下一步动作」。
 * 未蒸馏的 Laya 在 7 动作全集上接近噪声(偏爱 hard_drop → 原地锁死),
 * 约束到 2~3 个都朝目标推进的选项后,选哪个都不会变坏。
 */
export function candidateActions(snap: Snapshot, plan: PlanTarget): Action[] {
  const d = shortestRotation(snap.pieceRot, plan.placement.rot);
  const dx = plan.placement.x - snap.pieceX;
  const acts: Action[] = [];
  if (d > 0) acts.push('rotate');
  if (d < 0) acts.push('rotate_ccw');
  if (dx > 0) acts.push('shift_right');
  if (dx < 0) acts.push('shift_left');
  if (acts.length === 0) return ['hard_drop'];
  return acts;
}

/** 生成 laya-server 的 questions(reflex:仅候选动作单题,控制每步延迟)。 */
export function buildQuestions(candidates: readonly Action[]): Record<string, unknown> {
  const criteria: Record<string, string> = {};
  for (const a of candidates) criteria[a] = ACTION_HINTS[a];
  return {
    action: {
      type: 'choice',
      instructions:
        'Tetris reflex step. The board is 20 rows top-first, hex encoded: each row is 10 cells, 4 hex chars, bit 9 (MSB) = leftmost column, bit set = filled. The falling piece, plan target and board pressure are given. Choose exactly one next action.',
      criteria,
    },
  };
}

/** 棋盘 hex 编码:每行 10 格 → 4 个 hex 字符(bit9=最左列),20 行空格连接。
 * 相比 210 字符的点阵文本 token 数大减,控制每步推理延迟。 */
function boardToHex(board: Board): string {
  const rows: string[] = [];
  for (let y = 0; y < ROWS; y += 1) {
    let bits = 0;
    for (let x = 0; x < COLS; x += 1) {
      if ((board[y * COLS + x] ?? 0) !== 0) bits |= 1 << (COLS - 1 - x);
    }
    rows.push(bits.toString(16).padStart(4, '0'));
  }
  return rows.join(' ');
}

/** 把快照(含可选规划目标)文本化为 state。 */
export function buildState(
  snap: Snapshot,
  plan: PlanTarget | null,
  mode: DriveMode,
): Record<string, unknown> {
  const f = features(snap.board, 0);
  const height = stackHeight(snap.board);
  const state: Record<string, unknown> = {
    board_hex: boardToHex(snap.board),
    falling_piece: `${pieceName(snap.pieceId)} rot=${snap.pieceRot} x=${snap.pieceX} y=${snap.pieceY}`,
    next_piece: pieceName(snap.nextPieceId),
    board_pressure: `stack_height=${height}/20 holes=${f.holes} bumpiness=${colBumpiness(snap.board)}`,
  };
  if (mode === 'reflex' && plan?.placement) {
    const p = plan.placement;
    const rotDelta = shortestRotation(snap.pieceRot, p.rot);
    state.plan_target =
      `place ${plan.pieceName} at rot=${p.rot} x=${p.x} (from rot=${snap.pieceRot} x=${snap.pieceX}: ` +
      `${rotDelta === 0 ? 'rotation done' : rotDelta > 0 ? `rotate ${rotDelta}x clockwise` : `rotate ${-rotDelta}x ccw`}; ` +
      `${p.x === snap.pieceX ? 'column done' : p.x > snap.pieceX ? `shift_right ${p.x - snap.pieceX}` : `shift_left ${snap.pieceX - p.x}`})`;
  } else {
    state.plan_target = '(none - pure reflex, judge the board yourself)';
  }
  return state;
}

function colBumpiness(board: Board): number {
  const heights: number[] = [];
  for (let x = 0; x < 10; x += 1) {
    let h = 0;
    for (let y = 0; y < 20; y += 1) {
      if (board[y * 10 + x] !== 0) {
        h = 20 - y;
        break;
      }
    }
    heights.push(h);
  }
  let bump = 0;
  for (let i = 0; i < heights.length - 1; i += 1) {
    bump += Math.abs((heights[i] ?? 0) - (heights[i + 1] ?? 0));
  }
  return bump;
}

function shortestRotation(from: number, to: number): number {
  let d = (((to - from) % 4) + 4) % 4;
  if (d > 2) d -= 4;
  return d;
}

type PredictAnswer = {
  answers?: Record<
    string,
    {
      type?: string;
      choice?: string;
      score?: number;
      noul?: number;
      confidence?: number;
      probabilities?: Record<string, number>;
    }
  >;
};

export type BrainOptions = {
  endpoint?: string;
  mode: DriveMode;
  gateExecute?: number;
  gateEscalate?: number;
};

/** 反射脑:decide() 一次 = 一轮感知(含门控)。 */
export class LayaBrain {
  private endpoint: string;
  mode: DriveMode;
  /** 门控阈值(UI 实时可调;默认按游戏 OOD 域实测校准)。 */
  gateExecute: number;
  gateEscalate: number;
  /** RE_SENSE 连续计数(执行任意动作后归零)。 */
  private reSensed = 0;

  constructor(opts: BrainOptions) {
    this.endpoint = opts.endpoint ?? ENDPOINT_DEFAULT;
    this.mode = opts.mode;
    this.gateExecute = opts.gateExecute ?? GATE_EXECUTE;
    this.gateEscalate = opts.gateEscalate ?? GATE_ESCALATE;
  }

  reset(): void {
    this.reSensed = 0;
  }

  /** 一轮决策:内部完成 RE_SENSE 重问与 ESCALATE 升级。solver 模式不触网。 */
  async decide(snap: Snapshot, signal?: AbortSignal): Promise<Decision> {
    if (this.mode === 'solver') {
      const plan = planTarget(snap);
      return {
        action: 'hard_drop',
        conf: 1,
        gate: 'EXECUTE',
        latencyMs: 0,
        probs: {},
        danger: 0,
        plan: plan?.placement ?? null,
        sensed: 1,
      };
    }

    const plan = this.mode === 'reflex' ? planTarget(snap) : null;
    const candidates = plan ? candidateActions(snap, plan) : ACTIONS;
    const started = performance.now();
    const first = await this.ask(snap, plan, candidates, signal);
    let best: Decision = this.toDecision(first, snap, plan, candidates, 1);
    best.latencyMs = performance.now() - started;

    if (best.gate === 'RE_SENSE' && this.reSensed + 1 < MAX_RE_SENSE) {
      this.reSensed += 1;
      const again = await this.ask(snap, plan, candidates, signal);
      best = this.toDecision(again, snap, plan, candidates, 2);
      best.latencyMs = performance.now() - started;
    }
    if (best.gate !== 'ESCALATE') this.reSensed = 0;
    return best;
  }

  private toDecision(
    raw: PredictAnswer,
    snap: Snapshot,
    plan: PlanTarget | null,
    candidates: readonly Action[],
    sensed: number,
  ): Decision {
    const actionAns = raw.answers?.action ?? {};
    const inSet = (a: string | undefined): a is Action =>
      a !== undefined && (candidates as readonly string[]).includes(a);
    // 候选集过滤:Laya 选到集外(或没选)时,按其在候选上的概率回退。
    let action: Action = inSet(actionAns.choice) ? actionAns.choice : candidates[0]!;
    if (!inSet(actionAns.choice)) {
      let bestP = -1;
      for (const c of candidates) {
        const p = Number(actionAns.probabilities?.[c]) || 0;
        if (p > bestP) {
          bestP = p;
          action = c;
        }
      }
    }
    const conf = Number(actionAns.confidence ?? 0);
    const gate: Gate =
      conf >= this.gateExecute ? 'EXECUTE' : conf >= this.gateEscalate ? 'RE_SENSE' : 'ESCALATE';
    const decision: Decision = {
      action,
      conf,
      gate,
      latencyMs: 0,
      probs: {},
      danger: 0,
      plan: null,
      sensed,
    };
    for (const [k, v] of Object.entries(actionAns.probabilities ?? {})) {
      if ((ACTIONS as readonly string[]).includes(k)) decision.probs[k as Action] = Number(v);
    }
    if (gate === 'ESCALATE') {
      decision.plan = (plan ?? planTarget(snap))?.placement ?? null;
      decision.action = 'hard_drop';
    }
    return decision;
  }

  private async ask(
    snap: Snapshot,
    plan: PlanTarget | null,
    candidates: readonly Action[],
    signal?: AbortSignal,
  ): Promise<PredictAnswer> {
    const res = await fetch(`${this.endpoint}/predict`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        state: buildState(snap, plan, this.mode),
        questions: buildQuestions(candidates),
      }),
      signal,
    });
    if (!res.ok) throw new Error(`laya-server ${res.status}`);
    return (await res.json()) as PredictAnswer;
  }
}

/** 求解器直控的执行检查(reflex 模式下用于把"已到位"翻译成 hard_drop)。 */
export function atTarget(snap: Snapshot, plan: PlanTarget | null): boolean {
  if (!plan?.placement) return false;
  return snap.pieceRot === plan.placement.rot && snap.pieceX === plan.placement.x;
}
