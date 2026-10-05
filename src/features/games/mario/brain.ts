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
import { getPolicy, type ObsExtraKey } from './policy';
import {
  ACTIONS,
  ACTION_HINTS,
  nextHazards,
  nextIntent,
  type Action,
  type Intent,
} from './planner';
import { COLS, GROUND_ROW, ROWS, TILE, SOLID, type World } from './world1-1';

export type DriveMode = 'human' | 'reflex' | 'pure' | 'autopilot';
export type Gate = 'EXECUTE' | 'RE_SENSE' | 'ESCALATE';

/** 门控阈值:与 Tetris 同款起点;OOD 域必须实测重校(docs/11 §6),UI 滑杆可调。 */
export const GATE_EXECUTE = 0.16;
export const GATE_ESCALATE = 0.12;
const MAX_RE_SENSE = 2;

export const ENDPOINT_DEFAULT = 'http://127.0.0.1:10230';

/** /predict 单次请求超时（ms）。sidecar 半死时必须让 fetch settle。 */
export const PREDICT_TIMEOUT_MS = 3000;

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

/** 威胁清单,named 格式:名+距离(现行文案)。 */
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

/** 威胁清单,rows 格式(docs/15 §6.4):瓦距+行号,补同行纵向关系(veto 同排判据)。 */
export function threatRows(s: GameState, world: World, lookPx = 176): string {
  const parts: string[] = [];
  const hazards = nextHazards(s, world, lookPx);
  const front = s.mario.x + 14;
  for (const h of hazards) {
    const d = Math.ceil((h.startX - front) / TILE);
    if (h.kind === 'gap') {
      const w = Math.round((h.endX - h.startX) / TILE);
      parts.push(`gap d${d} w${w} r${GROUND_ROW}`);
    } else if (h.kind === 'pipe') {
      const hgt = Number(/h=(\d+)/.exec(h.label)?.[1] ?? 2);
      parts.push(`pipe d${d} h${hgt} r${GROUND_ROW - hgt}`);
    } else if (h.kind === 'goomba') {
      const g = (h as unknown as { ref?: { y?: number } }).ref;
      const row = g?.y !== undefined ? Math.floor(g.y / TILE) : GROUND_ROW;
      parts.push(`goomba d${d} r${row}`);
    } else parts.push(`step d${d} r${GROUND_ROW}`);
    if (parts.length >= 3) break;
  }
  return parts.length > 0 ? parts.join(' | ') : 'none';
}

/** profile 数字串 RLE:`13*12 9*2 5`(游程 ≥2 折叠,计数无损)。 */
export function rleProfile(nums: number[]): string {
  const parts: string[] = [];
  let i = 0;
  while (i < nums.length) {
    let j = i;
    while (j < nums.length && nums[j] === nums[i]) j += 1;
    const run = j - i;
    parts.push(run >= 2 ? `${nums[i]}*${run}` : String(nums[i]));
    i = j;
  }
  return parts.join(' ');
}

/** 附加状态字段(brain 无历史,由 driver 传入,docs/15 §6.3)。 */
export type ObsExtra = Partial<Record<ObsExtraKey, string | number>>;

/**
 * v3-obs 编码:压 token 的直接手段(实测延迟 ≈ 0.8ms/token,i9-14900HX)。
 * 规则:
 * - profile 从"逐列数字"改成 RLE 游程(`13*12 9*2 5`),无损但省 60%+ token;
 * - 姿态只留 3 个字符量纲(vx 档位 / 空中 / 朝向),小数点全部删掉;
 * - 进度只留百分比 + 关键计数,删掉 `x=1234/3392` 这种可由百分比反推的冗余;
 * - 目标编码版本号随数据落盘(便于事后消融某一段文本的贡献)。
 * 目标:常态 ≤125 token(≈100ms 档),给 10Hz 决策频率留出余量。
 */
export function encodeState(
  s: GameState,
  world: World,
  intent: Intent | null,
  mode: DriveMode,
  extra?: ObsExtra,
): Record<string, unknown> {
  // 观测件按策略沙盒裁剪(docs/14 §4 + docs/15 §6.4):agent 可消融任何观测与文案变体
  const P = getPolicy();
  const pct = Math.round((s.maxX / world.worldWidthPx) * 100);
  const state: Record<string, unknown> = { encoding: 'v3-obs' };
  if (P.obsProgressStyle === 'full') {
    state.p = `${pct}% c${s.coinCount} t${s.timeUnits} a${s.attempts}`;
  } else if (P.obsProgressStyle === 'pct') {
    state.p = `${pct}% c${s.coinCount}`;
  }
  state.prof = rleProfile(profile16(s, P.obsProfileCols));
  if (P.obsThreatFormat === 'named') state.th = threatText(s, world, P.obsThreatsLookPx);
  else if (P.obsThreatFormat === 'rows') state.th = threatRows(s, world, P.obsThreatsLookPx);
  if (P.obsIncludePose) state.m = poseShort(s);
  if (s.mario.power !== 'small') state.pw = s.mario.power === 'fire' ? 'F' : 'B';
  if (extra && P.obsStateExtra.length > 0) {
    const label: Record<ObsExtraKey, string> = {
      lastAction: 'last',
      heldTicks: 'held',
      stallTicks: 'stall',
    };
    const parts: string[] = [];
    for (const key of P.obsStateExtra) {
      const v = extra[key];
      if (v === undefined) continue;
      parts.push(`${label[key]}=${v}`);
    }
    if (parts.length > 0) state.ctx = parts.join(' ');
  }
  state.sub =
    mode === 'reflex' && intent && P.obsIncludeSubgoal
      ? `${intent.type} ${intent.note} [${intent.candidates.join(' ')}]`
      : '(none)';
  return state;
}

/** 位姿超短文案:`v2 a0 R`(速度档位 / 空中 / 朝向)——小数全部丢弃。 */
function poseShort(s: GameState): string {
  const m = s.mario;
  const v = Math.min(9, Math.round(Math.abs(m.vx)));
  return `v${v} a${m.onGround ? 0 : 1} ${m.face === 1 ? 'R' : 'L'}`;
}

/** terse 提示文案(代码注册变体,≤3 词)。 */
const TERSE_HINTS: Record<Action, string> = {
  idle: 'wait',
  left: 'walk left',
  right: 'walk right',
  run_left: 'run left',
  run_right: 'run right',
  jump: 'jump up',
  jump_left: 'jump left',
  jump_right: 'short jump right',
  jump_run_right: 'run-jump right',
  jump_run_left: 'run-jump left',
  fire_right: 'walk right + fire',
};

/** 指令模板变体(代码注册;{n} 插 obsProfileCols,防 concise 在小列数时说谎)。 */
const INSTRUCTIONS: Record<string, (n: number) => string> = {
  // 实测(docs/16 §4.3):指令文案是 token 的大头之一。把图例压进半句话
  // 比展开写省 ~30 token,且不影响零样本可用性(Laya 本来就靠 subgoal)。
  default: (n) =>
    `Pick one action. prof=RLE first solid row of next ${n} tiles (13=ground 15=pit). th=hazards.`,
  concise: (n) => `Pick one action. prof=${n} tiles ahead (13 ground, 15 pit).`,
  checklist: (n) =>
    `Pick one action. 1 th gap/pipe -> run-jump right. 2 th enemy -> jump. 3 else run right. prof=${n} tiles.`,
};

/** questions:仅候选动作单题;提示/指令文案随消息设计策略选型(docs/15 §6.4)。 */
export function buildQuestions(candidates: readonly Action[]): Record<string, unknown> {
  const P = getPolicy();
  const criteria: Record<string, string> = {};
  for (const a of candidates) {
    criteria[a] =
      P.obsHintStyle === 'full'
        ? ACTION_HINTS[a]
        : P.obsHintStyle === 'terse'
          ? TERSE_HINTS[a]
          : '';
  }
  const n = P.obsProfileCols;
  const make = INSTRUCTIONS[P.obsInstructionVariant] ?? INSTRUCTIONS.default!;
  return {
    action: { type: 'choice', instructions: make(n), criteria },
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
  /** 编码后的 state(遥测/UI STATE 预览/分析载荷③用,docs/15 §6.3)。 */
  stateJson: string;
  /** 编码后的 questions(收发明细用;RE_SENSE 重发同一份)。 */
  questionsJson: string;
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
  async decide(s: GameState, world: World, extra?: ObsExtra): Promise<Decision> {
    const intent = this.mode === 'reflex' ? nextIntent(s, world) : null;
    const candidates = intent ? intent.candidates : [...ACTIONS];
    const state = encodeState(s, world, intent, this.mode, extra);
    const questions = buildQuestions(candidates); // RE_SENSE 重发同一份,循环外构造一次

    const started = performance.now();
    let answer = await this.ask(state, questions);
    let sensed = 1;
    let conf = Number(answer.answers?.action?.confidence ?? 0);
    while (
      applyGate(conf, this.gateExecute, this.gateEscalate) === 'RE_SENSE' &&
      sensed < MAX_RE_SENSE
    ) {
      sensed += 1;
      answer = await this.ask(state, questions);
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
      questionsJson: JSON.stringify(questions),
      escalateManeuver: gate === 'ESCALATE',
    };
  }

  private async ask(
    state: Record<string, unknown>,
    questions: Record<string, unknown>,
  ): Promise<PredictAnswer> {
    // 超时是硬要求（分析 D2/P1）：sidecar 半死时 fetch 永不 settle，
    // driver 的 inFlight 会永为 true，整局决策永久停摆。10Hz 决策频率下
    // 3s 足够覆盖实测 240ms 档（docs/12 §3.6）的数十倍余量。
    const res = await this.fetchImpl(`${this.endpoint}/predict`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ state, questions }),
      signal: AbortSignal.timeout(PREDICT_TIMEOUT_MS),
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
