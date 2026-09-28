/**
 * Laya 决策端(docs/11 §6):把马里奥状态编码成 laya-server 的
 * {state, questions},再做置信度门控三分支(EXECUTE / RE_SENSE / ESCALATE)。
 *
 * 编码 v2-obs:进度 + 马里奥位姿 + 前方 N 列首个实心行(profile,列数由
 * policy.obsProfileCols 控制,坑=15,地面=13,管道顶=11)+ 命名威胁清单
 * (前扫 policy.obsThreatsLookPx)+ subgoal(reflex 才有)。观测件全部可按
 * 策略沙盒消融(docs/14 §4),让 Qwen 演化"喂什么数据给 Laya"。
 * 目标 ≤300 token,贴实测 240ms 档(docs/12 §3.6)。
 */

import type { GameState } from './engine';
import { getPolicy } from './policy';
import {
  ACTIONS,
  ACTION_HINTS,
  nextHazards,
  nextIntent,
  type Action,
  type Intent,
} from './planner';
import { COLS, ROWS, TILE, SOLID, type World } from './world1-1';

export type DriveMode = 'human' | 'reflex' | 'pure' | 'autopilot';
export type Gate = 'EXECUTE' | 'RE_SENSE' | 'ESCALATE';

/** 门控阈值:与 Tetris 同款起点;OOD 域必须实测重校(docs/11 §6),UI 滑杆可调。 */
export const GATE_EXECUTE = 0.16;
export const GATE_ESCALATE = 0.12;
const MAX_RE_SENSE = 2;

export const ENDPOINT_DEFAULT = 'http://127.0.0.1:10230';

export function applyGate(conf: number, gateExecute: number, gateEscalate: number): Gate {
  return conf >= gateExecute ? 'EXECUTE' : conf >= gateEscalate ? 'RE_SENSE' : 'ESCALATE';
}

/** 前方 cols 列的首个实心行(0..14;无实心 = 15)。地面=13,管顶 h2=11,砖行=9。 */
export function profile16(s: GameState, cols = 16): number[] {
  const base = Math.floor(s.mario.x / TILE) + 1;
  const out: number[] = [];
  for (let i = 0; i < cols; i += 1) {
    const c = base + i;
    let top = ROWS;
    if (c >= 0 && c < COLS) {
      for (let r = 0; r < ROWS; r += 1) {
        if (SOLID.has(s.tiles[r * COLS + c] ?? 0)) {
          top = r;
          break;
        }
      }
    }
    out.push(top);
  }
  return out;
}

/** 命名威胁清单(文本化直接给结论线索,docs/11 §6)。 */
export function threatText(s: GameState, world: World, lookPx = 176): string {
  const parts: string[] = [];
  const hazards = nextHazards(s, world, lookPx);
  const front = s.mario.x + 14;
  for (const h of hazards) {
    const dx = Math.round(h.startX - front);
    if (h.kind === 'gap') {
      const w = Math.round((h.endX - h.startX) / TILE);
      parts.push(`gap dx=${Math.round(dx / TILE)}t w=${w}`);
    } else if (h.kind === 'pipe') parts.push(`pipe dx=${dx}px ${h.label}`);
    else if (h.kind === 'goomba') parts.push(`goomba dx=${dx}px`);
    else parts.push(`step dx=${dx}px ${h.label}`);
    if (parts.length >= 3) break;
  }
  return parts.length > 0 ? parts.join(' | ') : 'none';
}

function pose(s: GameState): string {
  const m = s.mario;
  const v = `${m.vx >= 0 ? '+' : ''}${m.vx.toFixed(1)}/${m.vy >= 0 ? '+' : ''}${m.vy.toFixed(1)}`;
  return `vx/vy=${v} on_ground=${m.onGround ? 1 : 0} face=${m.face === 1 ? 'R' : 'L'}`;
}

/** 把状态文本化为 laya-server 的 state(编码模板版本号随数据落盘,便于消融)。 */
export function encodeState(
  s: GameState,
  world: World,
  intent: Intent | null,
  mode: DriveMode,
): Record<string, unknown> {
  // 观测件按策略沙盒裁剪(docs/14 §4):agent 可消融任何观测,纪律层评审效果
  const P = getPolicy();
  const pct = Math.round((s.maxX / world.worldWidthPx) * 100);
  const state: Record<string, unknown> = {
    encoding: 'v2-obs',
    progress: `x=${Math.round(s.mario.x)}/${world.worldWidthPx} ${pct}% coins=${s.coinCount} time=${s.timeUnits} attempt=${s.attempts}`,
    profile: profile16(s, P.obsProfileCols).join(' '),
    threats: threatText(s, world, P.obsThreatsLookPx),
  };
  if (P.obsIncludePose) state.mario = pose(s);
  state.subgoal =
    mode === 'reflex' && intent && P.obsIncludeSubgoal
      ? `${intent.type} ${intent.note} -> candidates [${intent.candidates.join(', ')}]`
      : '(none - pure reflex, judge the frame yourself)';
  return state;
}

/** questions:仅候选动作单题(控延迟,docs/11 §6;第二题 threat 默认关闭)。 */
export function buildQuestions(candidates: readonly Action[]): Record<string, unknown> {
  const criteria: Record<string, string> = {};
  for (const a of candidates) criteria[a] = ACTION_HINTS[a];
  return {
    action: {
      type: 'choice',
      instructions:
        'Mario reflex step. profile16 = first solid tile row of the 16 tiles ahead ' +
        '(13 = ground level, 15 = open pit, 9 = floating block row). Threats list named ' +
        'hazards with distance. Choose exactly one next action.',
      criteria,
    },
  };
}

type PredictAnswer = {
  answers?: Record<
    string,
    { choice?: string; confidence?: number; probabilities?: Record<string, number> }
  >;
};

export type Decision = {
  action: Action;
  conf: number;
  gate: Gate;
  latencyMs: number;
  probs: Partial<Record<Action, number>>;
  sensed: number;
  candidates: Action[];
  note: string;
  /** 编码后的 state(遥测/UI STATE 预览用)。 */
  stateJson: string;
  escalateManeuver: boolean;
};

type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

export type BrainOptions = {
  endpoint?: string;
  gateExecute?: number;
  gateEscalate?: number;
  fetchImpl?: FetchLike;
};

export class MarioBrain {
  mode: DriveMode;
  gateExecute: number;
  gateEscalate: number;
  private endpoint: string;
  private fetchImpl: FetchLike;
  private reSensed = 0;

  constructor(opts: BrainOptions = {}) {
    this.endpoint = opts.endpoint ?? ENDPOINT_DEFAULT;
    this.gateExecute = opts.gateExecute ?? GATE_EXECUTE;
    this.gateEscalate = opts.gateEscalate ?? GATE_ESCALATE;
    this.mode = 'reflex';
    // fetch 必须解绑 this 调用:作为实例方法调用会 Illegal invocation
    // (WebView2/Chromium 强制 receiver 为 window);tests 注入的 mock 不受影响。
    this.fetchImpl = opts.fetchImpl ?? ((input, init) => fetch(input, init));
  }

  reset(): void {
    this.reSensed = 0;
  }

  /** 一轮决策:reflex 用规划器候选集,pure 用全集;RE_SENSE 一次防抖。 */
  async decide(s: GameState, world: World): Promise<Decision> {
    const intent = this.mode === 'reflex' ? nextIntent(s, world) : null;
    const candidates = intent ? intent.candidates : [...ACTIONS];
    const state = encodeState(s, world, intent, this.mode);

    const started = performance.now();
    let answer = await this.ask(state, candidates);
    let sensed = 1;
    let conf = Number(answer.answers?.action?.confidence ?? 0);
    while (
      applyGate(conf, this.gateExecute, this.gateEscalate) === 'RE_SENSE' &&
      sensed < MAX_RE_SENSE
    ) {
      sensed += 1;
      answer = await this.ask(state, candidates);
      conf = Number(answer.answers?.action?.confidence ?? 0);
    }
    const latencyMs = performance.now() - started;

    // 候选集过滤 + 概率回退(未蒸馏 Laya 选到集外时,取候选上的最大概率)
    const inSet = (a: string | undefined): a is Action =>
      a !== undefined && (candidates as readonly string[]).includes(a);
    let action: Action = inSet(answer.answers?.action?.choice)
      ? answer.answers!.action!.choice!
      : candidates[0]!;
    if (!inSet(answer.answers?.action?.choice)) {
      let bestP = -1;
      for (const c of candidates) {
        const p = Number(answer.answers?.action?.probabilities?.[c]) || 0;
        if (p > bestP) {
          bestP = p;
          action = c;
        }
      }
    }
    const probs: Partial<Record<Action, number>> = {};
    for (const [k, v] of Object.entries(answer.answers?.action?.probabilities ?? {})) {
      if ((ACTIONS as readonly string[]).includes(k)) probs[k as Action] = Number(v);
    }
    const gate = applyGate(conf, this.gateExecute, this.gateEscalate);
    if (gate !== 'RE_SENSE') this.reSensed = 0;

    return {
      action,
      conf,
      gate,
      latencyMs,
      probs,
      sensed,
      candidates,
      note: intent ? `${intent.type} ${intent.note}` : 'pure',
      stateJson: JSON.stringify(state),
      escalateManeuver: gate === 'ESCALATE',
    };
  }

  private async ask(
    state: Record<string, unknown>,
    candidates: readonly Action[],
  ): Promise<PredictAnswer> {
    const res = await this.fetchImpl(`${this.endpoint}/predict`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ state, questions: buildQuestions(candidates) }),
    });
    if (!res.ok) throw new Error(`laya-server ${res.status}`);
    return (await res.json()) as PredictAnswer;
  }
}

/** 服务探活(只读;游戏页绝不代启,docs/11 §1.1)。 */
export async function checkEndpoint(
  url: string,
  fetchImpl: FetchLike = fetch,
  timeoutMs = 1500,
): Promise<boolean> {
  try {
    const res = await fetchImpl(url, { signal: AbortSignal.timeout(timeoutMs) });
    return res.ok;
  } catch {
    return false;
  }
}
