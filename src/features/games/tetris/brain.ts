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
  boardToText,
  dropY,
  features,
  pieceName,
  solvePlacements,
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

/** System 2 规划:给当前 piece 找最优放置(棋盘已满时可能为 null)。 */
export function planTarget(snap: Snapshot): PlanTarget | null {
  const best = solvePlacements(snap.board, snap.pieceId)[0];
  if (!best) return null;
  return {
    placement: best,
    pieceName: pieceName(snap.pieceId),
  };
}

/** 生成 laya-server 的 questions(动作选择 + 局面危险度)。 */
export function buildQuestions(): Record<string, unknown> {
  return {
    action: {
      type: 'choice',
      instructions:
        'Tetris reflex step. The board is shown top row first, "." empty "#" filled. The falling piece, the plan target (when present) and board pressure are given. Choose exactly one next action.',
      criteria: {
        shift_left: 'Move the falling piece one column left; use when target column is to the left',
        shift_right:
          'Move the falling piece one column right; use when target column is to the right',
        rotate: 'Rotate clockwise; use when target rotation is ahead of current rotation',
        rotate_ccw:
          'Rotate counter-clockwise; use when a single counter-clockwise step reaches target rotation',
        soft_drop: 'Move the piece down one row; use when close to the target row',
        hard_drop: 'Lock instantly; use only when current position already matches the target',
        wait: 'Do nothing this tick; use when nothing useful can be done',
      },
    },
    danger: {
      type: 'score',
      instructions: 'How dangerous is the board right now?',
      criteria: [
        'calm - low stack, no holes',
        'risky - rising stack or a few holes',
        'critical - holes deep or stack near top',
      ],
    },
  };
}

/** 把快照(含可选规划目标)文本化为 state。 */
export function buildState(
  snap: Snapshot,
  plan: PlanTarget | null,
  mode: DriveMode,
): Record<string, unknown> {
  const f = features(snap.board, 0);
  const height = stackHeight(snap.board);
  const rows = boardToText(snap.board);
  const state: Record<string, unknown> = {
    board: rows.join('\n'),
    falling_piece: `${pieceName(snap.pieceId)} rot=${snap.pieceRot} x=${snap.pieceX} y=${snap.pieceY}`,
    next_piece: pieceName(snap.nextPieceId),
    board_pressure: `stack_height=${height}/20 holes=${f.holes} bumpiness=${colBumpiness(snap.board)}`,
  };
  if (mode !== 'pure' && plan?.placement) {
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
    const started = performance.now();
    const first = await this.ask(snap, plan, signal);
    let best: Decision = this.toDecision(first, snap, plan, 1);
    best.latencyMs = performance.now() - started;

    if (best.gate === 'RE_SENSE' && this.reSensed + 1 < MAX_RE_SENSE) {
      this.reSensed += 1;
      const again = await this.ask(snap, plan, signal);
      best = this.toDecision(again, snap, plan, 2);
      best.latencyMs = performance.now() - started;
    }
    if (best.gate !== 'ESCALATE') this.reSensed = 0;
    return best;
  }

  private toDecision(
    raw: PredictAnswer,
    snap: Snapshot,
    plan: PlanTarget | null,
    sensed: number,
  ): Decision {
    const actionAns = raw.answers?.action ?? {};
    const dangerAns = raw.answers?.danger ?? {};
    const action = (ACTIONS as readonly string[]).includes(actionAns.choice ?? '')
      ? (actionAns.choice as Action)
      : 'wait';
    const conf = Number(actionAns.confidence ?? 0);
    const gate: Gate =
      conf >= this.gateExecute ? 'EXECUTE' : conf >= this.gateEscalate ? 'RE_SENSE' : 'ESCALATE';
    const decision: Decision = {
      action,
      conf,
      gate,
      latencyMs: 0,
      probs: {},
      danger: Number(dangerAns.score ?? 0),
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
    signal?: AbortSignal,
  ): Promise<PredictAnswer> {
    const res = await fetch(`${this.endpoint}/predict`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        state: buildState(snap, plan, this.mode),
        questions: buildQuestions(),
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

/** dropY 便捷再导出(供 UI 判定 hard_drop 是否安全)。 */
export { dropY };
