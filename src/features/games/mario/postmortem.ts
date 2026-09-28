/**
 * 复盘文档引擎(docs/12 §7 开发闭环的"分析"环节,纯函数、零依赖时钟)。
 *
 * 输入:一场会话的决策行/事件行/采样行(JSONL 或内存 ring)。
 * 输出:结构化报告(Report)+ 固定格式的 Markdown 总结文档。
 *
 * 设计约定:**假设与建议全部来自规则判定,不掺 LLM**——结论可测试、可复现。
 * 消费方分工:本地 Qwen(10230/17230 之外第三个本地服务,M6)读复盘做运行时
 * 监督;云端编码代理读复盘**改代码、重编译**(进化闭环的外圈)。
 */

import { TILE, WORLD_1_1 } from './world1-1';

export type PmDecisionRow = {
  action?: string;
  conf?: number;
  gate?: string;
  latencyMs?: number;
  col?: number;
  applied?: boolean;
  note?: string;
};

export type PmEventRow = {
  type: string;
  cause?: string;
  x?: number;
  maxX?: number;
  tick?: number;
  attempt?: number;
  score?: number;
};

export type PmSampleRow = { tick?: number; maxX?: number };

export type SessionInput = {
  id: string;
  mode?: string;
  decisions: PmDecisionRow[];
  events: PmEventRow[];
  samples: PmSampleRow[];
  final?: { attempts?: number; score?: number; coins?: number; maxX?: number; phase?: string };
  /** 本能否决次数:驱动层发现"直冲小怪"后用规划器机动接管一拍。 */
  vetoes?: number;
};

export type PostmortemReport = {
  id: string;
  outcome: 'win' | 'incomplete';
  attempts: number;
  score: number;
  maxXCol: number;
  progressPct: number;
  durationTicks: number;
  decisions: number;
  gates: { exec: number; reSense: number; escalate: number; stale: number; guard: number };
  execRate: number;
  escalateRate: number;
  staleRate: number;
  avgConf: number;
  p50LatencyMs: number;
  deaths: Array<{ cause: string; col: number; landmark: string }>;
  deathCauses: Array<{ cause: string; count: number }>;
  stallSites: Array<{ landmark: string; count: number; maxXCol: number }>;
  actionMix: Array<{ action: string; count: number; avgConf: number; execCount: number }>;
  vetoes: number;
  timeline: Array<{ sec: number; maxXCol: number }>;
  hypotheses: string[];
  suggestions: string[];
};

const WORLD = WORLD_1_1;

/** col → 关卡地标名(pipe 宽 2/gap/stair 单列/flag),失败定位的可读化。 */
export function landmarkOf(col: number): string {
  for (const p of WORLD.data.pipes) {
    if (col >= p.col && col <= p.col + 1) return `pipe@${p.col}(h${p.height})`;
  }
  for (const g of WORLD.gaps) {
    if (col >= g.startCol && col <= g.endCol) return `gap@${g.startCol}-${g.endCol}`;
  }
  // 旗杆优先:flagCol 198 同时是旗杆底座台阶,归属应为旗杆
  if (Math.abs(col - WORLD.data.flagCol) <= 2) return `flag@${WORLD.data.flagCol}`;
  for (const st of WORLD.data.stairs) {
    if (col === st.col) return `stair@${st.col}(h${st.height})`;
  }
  return `col${Math.round(col)}`;
}

/** 失败地点的地标归属:向上取整到"正在挑战"的地标(贴在 45.75 = 卡 pipe46)。 */
function siteOf(px: number | undefined): string {
  return landmarkOf(Math.ceil(px2col(px)));
}

const px2col = (px: number | undefined): number =>
  px === undefined ? 0 : Math.round((px / TILE) * 10) / 10;

function quantile(sorted: number[], q: number): number {
  if (sorted.length === 0) return 0;
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * q));
  return sorted[idx] ?? 0;
}

export function buildPostmortem(input: SessionInput): PostmortemReport {
  const applied = input.decisions.filter((d) => d.applied !== false);
  const stale = input.decisions.length - applied.length;
  const gates = { exec: 0, reSense: 0, escalate: 0, stale: 0, guard: 0 };
  for (const d of input.decisions) {
    if (d.applied === false) {
      gates.stale += 1;
      continue;
    }
    if (d.gate === 'EXECUTE') gates.exec += 1;
    else if (d.gate === 'RE_SENSE') gates.reSense += 1;
    else if (d.note === 'stall-guard') gates.guard += 1;
    else gates.escalate += 1;
  }
  const total = Math.max(1, input.decisions.length);
  const confs = applied.map((d) => d.conf ?? 0).sort((a, b) => a - b);
  const latencies = applied.map((d) => d.latencyMs ?? 0).sort((a, b) => a - b);
  const avgConf = confs.reduce((a, b) => a + b, 0) / Math.max(1, confs.length);

  // 死亡:cause × 地标
  const deaths = input.events
    .filter((e) => e.type === 'death')
    .map((e) => ({
      cause: e.cause ?? '?',
      col: px2col(e.x),
      landmark: siteOf(e.x),
    }));
  const causeMap = new Map<string, number>();
  for (const d of deaths) causeMap.set(d.cause, (causeMap.get(d.cause) ?? 0) + 1);
  const deathCauses = [...causeMap.entries()]
    .map(([cause, count]) => ({ cause, count }))
    .sort((a, b) => b.count - a.count);

  // 停滞:地标聚合
  const stallMap = new Map<string, { count: number; maxXCol: number }>();
  for (const e of input.events) {
    if (e.type !== 'stall') continue;
    const site = siteOf(e.maxX);
    const cur = stallMap.get(site) ?? { count: 0, maxXCol: px2col(e.maxX) };
    cur.count += 1;
    stallMap.set(site, cur);
  }
  const stallSites = [...stallMap.entries()]
    .map(([landmark, v]) => ({ landmark, count: v.count, maxXCol: v.maxXCol }))
    .sort((a, b) => b.count - a.count);

  // 动作混合:action × 次数 × 均置信
  const actMap = new Map<string, { count: number; confSum: number; execCount: number }>();
  for (const d of input.decisions) {
    const a = d.action ?? '?';
    const cur = actMap.get(a) ?? { count: 0, confSum: 0, execCount: 0 };
    cur.count += 1;
    cur.confSum += d.conf ?? 0;
    if (d.gate === 'EXECUTE' && d.applied !== false) cur.execCount += 1;
    actMap.set(a, cur);
  }
  const actionMix = [...actMap.entries()]
    .map(([action, v]) => ({
      action,
      count: v.count,
      avgConf: Math.round((v.confSum / v.count) * 100) / 100,
      execCount: v.execCount,
    }))
    .sort((a, b) => b.count - a.count);

  // 时间线:samples(约 5s 一个点)
  const timeline = input.samples
    .filter((sm) => sm.tick !== undefined && sm.maxX !== undefined)
    .map((sm) => ({ sec: Math.round((sm.tick ?? 0) / 60), maxXCol: px2col(sm.maxX) }));

  const win = input.events.some((e) => e.type === 'win');
  const maxX = input.final?.maxX ?? 0;
  const report: PostmortemReport = {
    id: input.id,
    outcome: win ? 'win' : 'incomplete',
    attempts: input.final?.attempts ?? 1,
    score: input.final?.score ?? 0,
    maxXCol: px2col(maxX),
    progressPct: Math.round((maxX / WORLD.worldWidthPx) * 1000) / 10,
    durationTicks:
      input.samples.length > 0 ? (input.samples[input.samples.length - 1]?.tick ?? 0) : 0,
    decisions: input.decisions.length,
    gates,
    execRate: Math.round((gates.exec / total) * 100) / 100,
    escalateRate: Math.round((gates.escalate / total) * 100) / 100,
    staleRate: Math.round((stale / total) * 100) / 100,
    avgConf: Math.round(avgConf * 1000) / 1000,
    p50LatencyMs: quantile(latencies, 0.5),
    deaths,
    deathCauses,
    stallSites,
    actionMix,
    vetoes: input.vetoes ?? 0,
    timeline,
    hypotheses: [],
    suggestions: [],
  };
  attachRules(report);
  return report;
}

/** 规则判定的根因假设与下一步建议(全部确定性,可单测)。 */
function attachRules(r: PostmortemReport): void {
  const h = r.hypotheses;
  const s = r.suggestions;
  if (r.escalateRate > 0.5) {
    h.push(
      `ESCALATE 率 ${(r.escalateRate * 100).toFixed(0)}%(avgConf ${r.avgConf})——置信度整体低于门控,System 1 在此状态分布上无判别力:域未蒸馏。`,
    );
    s.push('采集自驾教师轨迹 ≥10 局,对 Laya 做马里奥域蒸馏(M5);期间维持兜底保证演示。');
  }
  const topStall = r.stallSites[0];
  if (topStall && topStall.count >= 2) {
    if (topStall.landmark.startsWith('pipe')) {
      h.push(
        `停滞集中在 ${topStall.landmark}(${topStall.count} 次)——h3+ 管道需满速助跑,或起跳时机系统性偏早/偏晚。`,
      );
      s.push(
        `针对 ${topStall.landmark}:核对候选集含 jump_run_right;蒸馏采样以该段 dist∈[-8,16] 的帧加权。`,
      );
    } else if (topStall.landmark.startsWith('gap')) {
      h.push(`停滞集中在 ${topStall.landmark}(${topStall.count} 次)——坑缘起跳时机错误或助跑不足。`);
      s.push(
        `针对 ${topStall.landmark}:蒸馏采样以坑缘 dist∈[-14,10] 的帧加权;确认跳跃补全生效(空中不松键)。`,
      );
    } else {
      h.push(`停滞集中在 ${topStall.landmark}(${topStall.count} 次)。`);
    }
  }
  if (r.execRate > 0.25 && r.stallSites.length >= 2) {
    h.push(
      `EXECUTE 率 ${(r.execRate * 100).toFixed(0)}% 却多处停滞——"高置信错误动作"签名:动作语义与物理时机错位,state 缺少精确定位特征。`,
    );
    s.push('state 编码追加 hazardDist(ticks)与 self.vx;重训后对比同门控下 EXECUTE 命中率。');
  }
  const gapDeaths = r.deathCauses.find((d) => d.cause === 'pit');
  if (gapDeaths && gapDeaths.count >= 2) {
    h.push(`坠落坑 ${gapDeaths.count} 次——跳跃距离估计系统性不足(起跳过早)或空中被拍间截断。`);
    s.push('复核跳跃弧补全日志;蒸馏时坑缘帧标注"必须 run+jump"。');
  }
  if (r.staleRate > 0.1) {
    h.push(
      `stale 率 ${(r.staleRate * 100).toFixed(0)}%(p50 ${r.p50LatencyMs}ms)——决策延迟与死亡间隔同量级,拍在死亡后才落地。`,
    );
    s.push('intervalMs 上调 50-100ms,或死亡/重生后 500ms 静默再恢复发射。');
  }
  if (r.vetoes >= 5) {
    h.push(
      `本能否决 ${r.vetoes} 次——Laya 对近身小怪无反应(直冲),由驱动层 veto 接管。蒸馏后该数应单调下降。`,
    );
    s.push(
      '蒸馏采样把"同排小怪 dist∈(0,16] 且 self 未跳"的帧标签设为 jump;veto 率是蒸馏效果的第一指标。',
    );
  }
  if (
    r.outcome === 'incomplete' &&
    r.attempts >= 3 &&
    r.maxXCol < 60 &&
    !r.hypotheses.some((x) => x.includes('未蒸馏'))
  ) {
    h.push(
      `尝试 ${r.attempts} 次仅到 col ${r.maxXCol}——失败密度过高,当前 checkpoint 不具备该段能力。`,
    );
    s.push('回退目标:先以自驾对照确认关卡可通过,再蒸馏;不要在噪声上调门控。');
  }
  if (h.length === 0) {
    h.push('规则未命中显著模式——查看原始统计与时间线人工研判。');
  }
}

const pct = (v: number): string => `${(v * 100).toFixed(0)}%`;

/** 固定小节的 Markdown 总结文档(本地 agent 与云端模型共用格式)。 */
export function renderMarkdown(r: PostmortemReport): string {
  const lines: string[] = [];
  lines.push(`# Laya Jump 复盘 · ${r.id}`);
  lines.push('');
  lines.push(
    `- 结果:**${r.outcome === 'win' ? '通关' : '未通关'}** · 尝试 ${r.attempts} · score ${r.score} · 最远 col ${r.maxXCol}(${r.progressPct}%) · 时长 ${(r.durationTicks / 60).toFixed(0)}s`,
  );
  lines.push(
    `- 决策 ${r.decisions} 拍:EXECUTE ${pct(r.execRate)} / ESCALATE ${pct(r.escalateRate)} / RE-SENSE ${pct(r.gates.reSense / Math.max(1, r.decisions))} / stale ${pct(r.staleRate)} · 兜底拍 ${r.gates.guard} · veto ${r.vetoes} · avgConf ${r.avgConf} · p50 ${r.p50LatencyMs}ms`,
  );
  lines.push('');
  lines.push('## 失败分布');
  if (r.deathCauses.length === 0) lines.push('- 无死亡');
  for (const d of r.deathCauses) lines.push(`- 死亡 ${d.cause} ×${d.count}`);
  for (const st of r.stallSites)
    lines.push(`- 停滞 ${st.landmark} ×${st.count}(maxX col ${st.maxXCol})`);
  lines.push('');
  lines.push('## 死亡明细');
  for (const d of r.deaths.slice(0, 12)) lines.push(`- col ${d.col}(${d.landmark})${d.cause}`);
  if (r.deaths.length === 0) lines.push('- 无');
  lines.push('');
  lines.push('## 动作混合 Top5');
  for (const a of r.actionMix.slice(0, 5)) {
    lines.push(`- ${a.action} ×${a.count}(avgConf ${a.avgConf},EXECUTE ${a.execCount})`);
  }
  lines.push('');
  lines.push('## 时间线(每 5s 采样)');
  lines.push(
    r.timeline.length > 0
      ? r.timeline.map((t) => `${t.sec}s:${t.maxXCol}`).join(' → ')
      : '- 无采样',
  );
  lines.push('');
  lines.push('## 根因假设(规则判定)');
  for (const x of r.hypotheses) lines.push(`- ${x}`);
  lines.push('');
  lines.push('## 下一步实验建议');
  for (const x of r.suggestions) lines.push(`- ${x}`);
  lines.push('');
  lines.push('## 原始统计(JSON)');
  lines.push('```json');
  lines.push(JSON.stringify({ ...r, hypotheses: undefined, suggestions: undefined }));
  lines.push('```');
  return lines.join('\n');
}
