/**
 * 进化闭环显式状态机 + 分析调度器(docs/15 §3.1/§6.2)。
 *
 * 纯逻辑、零 IO、零 mario 依赖(lib/evolution-core 兼容写):UI(MarioLab)
 * 与无头闭环(e2e/mario-loop.ts)复用同一份——不再各写第三份隐式状态机。
 * effects(仿真节拍/HTTP/落盘)由宿主执行,这里只定义迁移与队列规则。
 */

// ================= 状态机(docs/15 §3.1 九状态表) =================

export type LoopPhase =
  | 'IDLE'
  | 'TRAINING'
  | 'POSTMORTEM'
  | 'TRIGGER'
  | 'QUEUED'
  | 'ANALYZING'
  | 'CANDIDATE_EVAL'
  | 'VERDICT'
  | 'PAUSED';

export type LoopEvent =
  | 'start'
  | 'beginRun'
  | 'runEnd'
  | 'plan'
  | 'enqueue'
  | 'dequeue'
  | 'propose'
  | 'skipAnalyzed'
  | 'evalStart'
  | 'verdict'
  | 'pause'
  | 'resume'
  | 'error'
  | 'reset';

const TRANSITIONS: Record<LoopPhase, Partial<Record<LoopEvent, LoopPhase>>> = {
  IDLE: { start: 'TRAINING', reset: 'IDLE' },
  TRAINING: { runEnd: 'POSTMORTEM', pause: 'PAUSED', error: 'IDLE' },
  POSTMORTEM: { plan: 'TRIGGER', error: 'IDLE' },
  TRIGGER: { enqueue: 'QUEUED', beginRun: 'TRAINING' }, // 无任务直接开下局
  QUEUED: { dequeue: 'ANALYZING', beginRun: 'TRAINING' }, // 队列空则开下局
  ANALYZING: {
    propose: 'CANDIDATE_EVAL', // 产出候选
    skipAnalyzed: 'QUEUED', // 空补丁/幂等跳过 → 下一任务
    beginRun: 'TRAINING', // 任务清空
    pause: 'PAUSED',
    error: 'QUEUED', // 失败重试/降级,不阻塞开新局
  },
  CANDIDATE_EVAL: { verdict: 'VERDICT', error: 'VERDICT', pause: 'PAUSED' },
  VERDICT: { beginRun: 'TRAINING', reset: 'IDLE' },
  PAUSED: { resume: 'TRAINING', reset: 'IDLE' }, // resume 由宿主回到记录的原相
};

/** 不可变相位快照:PAUSED 记录挂起前的相,resume 回原相。 */
export type LoopState = { phase: LoopPhase; prev?: LoopPhase };

export function transition(state: LoopState, event: LoopEvent): LoopState {
  if (event === 'pause' && state.phase !== 'PAUSED') {
    return { phase: 'PAUSED', prev: state.phase };
  }
  if (event === 'resume' && state.phase === 'PAUSED') {
    return { phase: state.prev ?? 'TRAINING' };
  }
  const next = TRANSITIONS[state.phase]?.[event];
  if (!next) return state; // 非法迁移:保持原相(调用方审计)
  return { phase: next };
}

// ================= 分析调度器(docs/15 §6.2) =================

export type SchedTask = {
  key: string;
  kind: 'death' | 'win' | 'plateau';
  attempt?: number;
  sig?: string | null;
  reason: string;
  priority: number;
  /** 已尝试次数(≤2,超限放弃并审计 fail) */
  attempts?: number;
};

export const QUEUE_CAP = 6;
export const MAX_TASK_ATTEMPTS = 2;
export const BREAKER_THRESHOLD = 3;

/**
 * 入队:同 key 合并(留最新);按优先级排序;溢出弹出最低优先级任务
 * 交由宿主落盘 backlog(不丢死亡)。返回 merged/overflow 供计数。
 */
export function scheduleTask(
  queue: SchedTask[],
  task: SchedTask,
  cap = QUEUE_CAP,
): { queue: SchedTask[]; merged: boolean; overflow: SchedTask | null } {
  const same = queue.find((t) => t.key === task.key);
  if (same) {
    return {
      queue: queue.map((t) => (t.key === task.key ? { ...task, attempts: t.attempts } : t)),
      merged: true,
      overflow: null,
    };
  }
  const next = [...queue, task].sort((a, b) => a.priority - b.priority);
  if (next.length <= cap) return { queue: next, merged: false, overflow: null };
  return { queue: next.slice(0, cap), merged: false, overflow: next[next.length - 1] ?? null };
}

/** 取下一任务:backlog(落盘)优先于内存队列;各自按优先级。 */
export function nextTask(backlog: SchedTask[], queue: SchedTask[]): SchedTask | null {
  const pick = (xs: SchedTask[]): SchedTask | null =>
    xs.length === 0 ? null : ([...xs].sort((a, b) => a.priority - b.priority)[0] ?? null);
  return pick(backlog) ?? pick(queue);
}

export function removeTask(xs: SchedTask[], key: string): SchedTask[] {
  return xs.filter((t) => t.key !== key);
}

/** 失败处置:attempts+1 ≤ 上限 → 可重试(返回新任务);否则 null(放弃并审计)。 */
export function taskFailed(t: SchedTask): SchedTask | null {
  const attempts = (t.attempts ?? 0) + 1;
  return attempts <= MAX_TASK_ATTEMPTS ? { ...t, attempts } : null;
}

/** 连续失败熔断(docs/15 R2/R4):探活成功由宿主调 reset 复位。 */
export class CircuitBreaker {
  private fails = 0;
  get tripped(): boolean {
    return this.fails >= BREAKER_THRESHOLD;
  }
  ok(): void {
    this.fails = 0;
  }
  fail(): boolean {
    this.fails += 1;
    return this.tripped;
  }
}
