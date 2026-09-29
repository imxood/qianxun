/**
 * 逐局数据聚合分析(docs/15 §9 数据留存)。
 *
 * 用法:npx tsx e2e/mario-run-report.ts [dataDir=~/.qianxun_dev/games/mario]
 * 输入:runs/index.jsonl(概览)+ runs/<runId>.json(明细,按需抽样)
 *      + evolution.jsonl(分析对账)+ state.json(计数器)
 * 输出:局数/胜率/趋势、死亡热区、门控健康、Qwen 链路对账、经验库健康。
 */
import fs from 'node:fs';
import path from 'node:path';
import { defaultDataDir } from './mario-loop';

type IndexRow = {
  ts: string;
  runId: string;
  mode: string;
  won: boolean;
  ticks: number;
  maxXCol: number;
  attempts: number;
  deaths: number;
  tasksPlanned: number;
  tasksOk: number;
};

type RunFile = {
  runId: string;
  ts: string;
  stats?: { decisions: number; exec: number; vetoes: number };
  policy?: Record<string, unknown>;
  report?: {
    outcome?: string;
    deathCauses?: Array<{ cause: string; count: number }>;
    deaths?: Array<{ cause: string; col: number; attempt: number }>;
    stallSites?: number[];
    thrashSites?: number[];
  };
};

function readJson<T>(p: string): T | null {
  try {
    return JSON.parse(fs.readFileSync(p, 'utf8')) as T;
  } catch {
    return null;
  }
}

function readJsonl<T>(p: string): T[] {
  try {
    return fs
      .readFileSync(p, 'utf8')
      .split('\n')
      .filter((l) => l.trim() !== '')
      .map((l) => {
        try {
          return JSON.parse(l) as T;
        } catch {
          return null;
        }
      })
      .filter((x): x is T => x !== null);
  } catch {
    return [];
  }
}

export function main(argv: string[]): void {
  const dir = argv[2] ?? defaultDataDir();
  const runsDir = path.join(dir, 'runs');
  const index = readJsonl<IndexRow>(path.join(dir, 'runs', 'index.jsonl'));
  const audit = readJsonl<Record<string, unknown>>(path.join(dir, 'evolution.jsonl'));
  const state = readJson<Record<string, unknown>>(path.join(dir, 'state.json'));
  const insightsRaw = fs.existsSync(path.join(dir, 'insights.md'))
    ? fs.readFileSync(path.join(dir, 'insights.md'), 'utf8')
    : '';

  if (index.length === 0) {
    console.log(`(无逐局归档:${runsDir} —— 先跑 e2e 或 UI 局末自动落盘)`);
  }
  const wins = index.filter((r) => r.won).length;
  const best = index.reduce((m, r) => Math.max(m, r.maxXCol), 0);
  console.log('==== 逐局归档概览 ====');
  console.log(
    `局数 ${index.length} · 胜率 ${index.length ? ((wins / index.length) * 100).toFixed(0) : 0}% · 最远 ${best.toFixed(1)} 列 · 平均 ${index.length ? (index.reduce((s, r) => s + r.ticks, 0) / index.length).toFixed(0) : 0} ticks`,
  );
  const byMode = new Map<string, IndexRow[]>();
  for (const r of index) {
    const arr = byMode.get(r.mode) ?? [];
    arr.push(r);
    byMode.set(r.mode, arr);
  }
  for (const [m, rows] of byMode) {
    const w = rows.filter((x) => x.won).length;
    console.log(
      `  ${m}: ${rows.length} 局 · 胜 ${w} · 最远 ${rows.reduce((a, r) => Math.max(a, r.maxXCol), 0).toFixed(1)} 列`,
    );
  }
  console.log(
    '近 10 局最远列:',
    index
      .slice(-10)
      .map((r) => r.maxXCol.toFixed(0))
      .join(' → '),
  );

  // 死亡热区(80 列一桶)
  const buckets = new Map<number, number>();
  const causes = new Map<string, number>();
  for (const r of index) {
    const f = readJson<RunFile>(path.join(runsDir, `${r.runId}.json`));
    for (const d of f?.report?.deaths ?? []) {
      buckets.set(Math.floor(d.col / 20) * 20, (buckets.get(Math.floor(d.col / 20)) ?? 0) + 1);
      causes.set(d.cause, (causes.get(d.cause) ?? 0) + 1);
    }
  }
  if (buckets.size > 0) {
    console.log('==== 死亡热区(20 列一桶) ====');
    for (const [b, n] of [...buckets].sort((x, y) => x[0] - y[0])) {
      console.log(`  col ${String(b).padStart(3)}+: ${'#'.repeat(n)} ${n}`);
    }
    console.log('死因:', [...causes].map(([c, n]) => `${c}×${n}`).join(' · '));
  }

  // 门控健康(抽样最近 5 局明细)
  const recent = index.slice(-5);
  let dec = 0;
  let exec = 0;
  let veto = 0;
  for (const r of recent) {
    const f = readJson<RunFile>(path.join(runsDir, `${r.runId}.json`));
    dec += f?.stats?.decisions ?? 0;
    exec += f?.stats?.exec ?? 0;
    veto += f?.stats?.vetoes ?? 0;
  }
  if (dec > 0) {
    console.log(
      `==== 门控健康(近 ${recent.length} 局) ==== 决策 ${dec} · 执行率 ${((exec / dec) * 100).toFixed(0)}% · 否决 ${veto}`,
    );
  }

  // Qwen 链路对账:审计 vs 计数器 vs 索引
  const analyzeOk = audit.filter((a) => a['action'] === 'analyze' && a['phase'] === 'ok').length;
  const analyzeFail = audit.filter(
    (a) => a['action'] === 'analyze' && a['phase'] === 'fail',
  ).length;
  const proposes = audit.filter((a) => a['action'] === 'propose').length;
  const commits = audit.filter((a) => a['action'] === 'commit').length;
  const rollbacks = audit.filter((a) => a['action'] === 'rollback').length;
  const counters = (state?.['counters'] as { qwen?: Record<string, number> } | undefined)?.qwen;
  console.log('==== Qwen 链路对账 ====');
  console.log(
    `审计:analyze ok ${analyzeOk} / fail ${analyzeFail} · 提案 ${proposes} · 转正 ${commits} · 回滚 ${rollbacks}`,
  );
  if (counters) {
    console.log(
      `计数器:死亡分析 ${counters['deathAnalyses']} · 提案 ${counters['patches']} · 转正 ${counters['commits']} · 回滚 ${counters['rollbacks']} · 失败 ${counters['failed']} · 超时 ${counters['timeouts']}`,
    );
  }
  const idxPlanned = index.reduce((s, r) => s + r.tasksPlanned, 0);
  const idxOk = index.reduce((s, r) => s + r.tasksOk, 0);
  if (idxPlanned > 0) console.log(`索引:分析任务 ${idxOk}/${idxPlanned} 成功`);

  // 经验库健康
  const lines = insightsRaw.split('\n').filter((l) => l.startsWith('- [iter'));
  const confirms = (insightsRaw.match(/‖确认@/g) ?? []).length;
  console.log(
    `==== 经验库 ==== ${lines.length} 条 · 近重复确认 ${confirms} 次 · ${insightsRaw.length} 字`,
  );
}

if (process.argv[1] && process.argv[1].replace(/\\/g, '/').includes('mario-run-report')) {
  main(process.argv);
}
