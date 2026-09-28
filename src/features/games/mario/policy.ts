/**
 * 策略沙箱(docs/12 §7 开发闭环的"本地 Agent 可动子空间")。
 *
 * 本地 Qwen agent 每局结束输出 policy_patch,只能改这里的字段;sanitize
 * 把一切钳制到安全值域——agent 自由迭代,但永远改不出坏不掉的范围。
 * 引擎/规划器/驱动运行时读取(经 setPolicy 热生效),无需重编译。
 */

export type Window = [number, number];

export type PolicyProfile = {
  /** 决策拍间隔 ms(150-600)。 */
  intervalMs: number;
  /** 门控:EXECUTE 下限(0.05-0.5)。 */
  gateExecute: number;
  /** 门控:ESCALATE 下限(0.02-0.3)。 */
  gateEscalate: number;
  /** 本能否决开关(直冲小怪时规划器接管一拍)。 */
  vetoEnabled: boolean;
  /** veto 触发距离 px(6-40)。 */
  vetoDistPx: number;
  /** 起跳触发窗(px,相对危险前沿):坑。 */
  gapWindow: Window;
  /** 起跳触发窗:管道。 */
  pipeWindow: Window;
  /** 起跳触发窗:小怪。 */
  goombaWindow: Window;
  /** 起跳触发窗:台阶。 */
  stairWindow: Window;
  /** 贴墙后撤时长 tick(10-60)。 */
  backoffTicks: number;
  /** 观测模式(docs/14 §4):profile 前瞻列数(8-24)。 */
  obsProfileCols: number;
  /** 观测模式:威胁清单前扫距离 px(96-288)。 */
  obsThreatsLookPx: number;
  /** 观测模式:是否携带 mario 位姿(vx/vy/onGround/face)。 */
  obsIncludePose: boolean;
  /** 观测模式:reflex 是否携带 subgoal 与候选集。 */
  obsIncludeSubgoal: boolean;
};

export const DEFAULT_POLICY: PolicyProfile = {
  intervalMs: 250,
  gateExecute: 0.16,
  gateEscalate: 0.12,
  vetoEnabled: true,
  vetoDistPx: 14,
  gapWindow: [-14, 10],
  pipeWindow: [2, 16],
  goombaWindow: [-8, 34],
  stairWindow: [-2, 10],
  backoffTicks: 26,
  obsProfileCols: 16,
  obsThreatsLookPx: 176,
  obsIncludePose: true,
  obsIncludeSubgoal: true,
};

const BOUNDS = {
  intervalMs: [150, 600],
  gateExecute: [0.05, 0.5],
  gateEscalate: [0.02, 0.3],
  vetoDistPx: [6, 40],
  backoffTicks: [10, 60],
  obsProfileCols: [8, 24],
  obsThreatsLookPx: [96, 288],
} as const;

const WINDOW_BOUNDS: Record<string, [number, number]> = {
  gapWindow: [-30, 40],
  pipeWindow: [-10, 40],
  goombaWindow: [-20, 60],
  stairWindow: [-10, 40],
};

let current: PolicyProfile = { ...DEFAULT_POLICY };

export function getPolicy(): PolicyProfile {
  return current;
}

export function setPolicy(p: PolicyProfile): void {
  current = { ...p };
}

export function resetPolicy(): void {
  current = { ...DEFAULT_POLICY };
}

function clamp(v: number, [lo, hi]: readonly [number, number]): number {
  return Math.min(hi, Math.max(lo, v));
}

/**
 * 沙箱校验:未知字段丢弃、越界钳制、窗口逆序自动交换。
 * 返回 issues 列出所有"改了但被钳制"的字段(agent 可从中学到值域)。
 */
export function sanitizePolicy(raw: unknown): {
  ok: boolean;
  policy: PolicyProfile;
  issues: string[];
} {
  const issues: string[] = [];
  const src = (typeof raw === 'object' && raw !== null ? raw : {}) as Record<string, unknown>;
  const out: PolicyProfile = { ...DEFAULT_POLICY };
  const num = (key: keyof typeof BOUNDS): void => {
    const v = src[key];
    if (typeof v !== 'number' || Number.isNaN(v)) return;
    const [lo, hi] = BOUNDS[key];
    const c = clamp(v, [lo, hi]);
    if (c !== v) issues.push(`${key}: ${v} → ${c}(钳制)`);
    (out as unknown as Record<string, number>)[key] = c;
  };
  num('intervalMs');
  num('gateExecute');
  num('gateEscalate');
  num('vetoDistPx');
  num('backoffTicks');
  num('obsProfileCols');
  num('obsThreatsLookPx');
  if (typeof src.vetoEnabled === 'boolean') out.vetoEnabled = src.vetoEnabled;
  if (typeof src.obsIncludePose === 'boolean') out.obsIncludePose = src.obsIncludePose;
  if (typeof src.obsIncludeSubgoal === 'boolean') out.obsIncludeSubgoal = src.obsIncludeSubgoal;
  // 观测几何量必须是整数(列数/像素)
  out.obsProfileCols = Math.round(out.obsProfileCols);
  out.obsThreatsLookPx = Math.round(out.obsThreatsLookPx);
  for (const [key, [lo, hi]] of Object.entries(WINDOW_BOUNDS)) {
    const v = src[key];
    if (!Array.isArray(v) || v.length !== 2) continue;
    let a = clamp(Number(v[0]), [lo, hi]);
    let b = clamp(Number(v[1]), [lo, hi]);
    if (Number.isNaN(a) || Number.isNaN(b)) continue;
    if (a > b) [a, b] = [b, a];
    const cur = out[key as 'gapWindow'] as Window;
    if (a !== cur[0] || b !== cur[1]) issues.push(`${key}: [${v[0]},${v[1]}] → [${a},${b}]`);
    out[key as 'gapWindow'] = [a, b];
  }
  // 不变式:ESCALATE 门 ≤ EXECUTE 门(压 ESCALATE,保住用户设定的 EXECUTE)
  if (out.gateEscalate > out.gateExecute) {
    issues.push(
      `gateEscalate ${out.gateEscalate} > gateExecute ${out.gateExecute} → 压到 ${out.gateExecute}`,
    );
    out.gateEscalate = out.gateExecute;
  }
  return { ok: issues.length === 0, policy: out, issues };
}

/** 从磁盘 JSON 载入并校验(node 侧)。 */
export function policyFromJson(raw: unknown): { policy: PolicyProfile; issues: string[] } {
  const { policy, issues } = sanitizePolicy(raw);
  return { policy, issues };
}
