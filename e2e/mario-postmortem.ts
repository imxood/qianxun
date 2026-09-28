/**
 * 复盘批量工具(docs/12 §7 开发闭环的"分析"环节,本地侧入口)。
 *
 * 扫描 laya-server 会话目录下的 laya-mario JSONL:
 * - 每局生成 `<id>.summary.md`(固定格式,规则判定,无 LLM)
 * - 汇总 `INDEX.md`:失败排行、跨会话死因/停滞聚合 —— 进化闭环的依据
 *
 * 用法:
 *   npx tsx e2e/mario-postmortem.ts [sessionsDir] [--force]
 * 目录缺省依次探测:%LOCALAPPDATA%\com.qianxun.desktop|com.qianxun.e2e\logs\laya-sessions、
 * var/mario/sessions、logs。
 * 消费方:云端编码代理(读 INDEX/summary → 改代码 → 重编译 → A/B);本地 Qwen(M6)。
 */

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import {
  buildPostmortem,
  renderMarkdown,
  type PmDecisionRow,
  type PmEventRow,
  type PmSampleRow,
  type SessionInput,
} from '../src/features/games/mario/postmortem';

function candidateDirs(): string[] {
  const argvDir = process.argv[2];
  const list = [
    argvDir,
    // 千寻数据根:release ~/.qianxun / debug ~/.qianxun_dev(paths.rs 约定)
    path.join(os.homedir(), '.qianxun', 'logs', 'laya-sessions'),
    path.join(os.homedir(), '.qianxun_dev', 'logs', 'laya-sessions'),
    path.resolve('var', 'mario', 'sessions'),
    path.resolve('logs'),
  ].filter((v): v is string => Boolean(v));
  return list;
}

function pickDir(): string | null {
  for (const dir of candidateDirs()) {
    try {
      if (fs.statSync(dir).isDirectory()) return dir;
    } catch {
      /* 下一个候选 */
    }
  }
  return null;
}

type Rows = {
  id: string;
  game: string;
  mode: string;
  decisions: PmDecisionRow[];
  events: PmEventRow[];
  samples: PmSampleRow[];
  final: SessionInput['final'];
  hasPostmortem: boolean;
};

function parseSession(file: string): Rows | null {
  const id = path.basename(file).replace(/\.jsonl$/i, '');
  const rows: Rows = {
    id,
    game: '?',
    mode: '?',
    decisions: [],
    events: [],
    samples: [],
    final: undefined,
    hasPostmortem: false,
  };
  const lines = fs.readFileSync(file, 'utf8').split('\n').filter(Boolean);
  for (const line of lines) {
    let j: Record<string, unknown>;
    try {
      j = JSON.parse(line) as Record<string, unknown>;
    } catch {
      continue; // 容忍尾部半行
    }
    const type = String(j.type ?? '');
    if (type === 'meta') {
      rows.game = String(j.game ?? '?');
      const meta = (j.meta ?? {}) as Record<string, unknown>;
      if (typeof meta.mode === 'string') rows.mode = meta.mode;
      continue;
    }
    if (type === 'postmortem') {
      rows.hasPostmortem = true;
      continue;
    }
    if (type === 'decision') {
      rows.decisions.push({
        action: j.action as string | undefined,
        conf: j.conf as number | undefined,
        gate: j.gate as string | undefined,
        latencyMs: j.latencyMs as number | undefined,
        col: j.col as number | undefined,
        applied: j.applied as boolean | undefined,
        note: j.note as string | undefined,
      });
    } else if (type === 'event') {
      rows.events.push({ ...(j as PmEventRow), type: String(j.event ?? 'unknown') });
    } else if (type === 'sample') {
      rows.samples.push({ tick: j.tick as number | undefined, maxX: j.maxX as number | undefined });
      rows.final = {
        attempts: (j.attempts as number | undefined) ?? rows.final?.attempts,
        score: (j.score as number | undefined) ?? rows.final?.score,
        maxX: (j.maxX as number | undefined) ?? rows.final?.maxX,
      };
    }
  }
  return rows;
}

function main(): void {
  const force = process.argv.includes('--force');
  const dir = pickDir();
  if (!dir) {
    console.error('未找到会话目录;传参:npx tsx e2e/mario-postmortem.ts <sessionsDir>');
    process.exit(1);
  }
  const files = fs.readdirSync(dir).filter((f) => f.endsWith('.jsonl'));
  if (files.length === 0) {
    console.error(`${dir} 下没有 JSONL 会话`);
    process.exit(1);
  }
  const indexRows: string[] = [];
  const causeTotals = new Map<string, number>();
  const stallTotals = new Map<string, number>();
  let generated = 0;
  let skipped = 0;
  for (const f of files) {
    const full = path.join(dir, f);
    const rows = parseSession(full);
    // 只复盘 mario 会话(tetris 等其它游戏的行结构不同,直接跳过)
    if (!rows || rows.game !== 'laya-mario') continue;
    if (rows.decisions.length + rows.events.length === 0) continue;
    const vetoes = rows.events.filter((e) => e.type === 'veto').length;
    const input: SessionInput = {
      id: rows.id,
      mode: rows.mode,
      decisions: rows.decisions,
      events: rows.events,
      samples: rows.samples,
      final: rows.final,
      vetoes,
    };
    const report = buildPostmortem(input);
    const md = renderMarkdown(report);
    fs.writeFileSync(path.join(dir, `${rows.id}.summary.md`), md, 'utf8');
    generated += 1;
    if (rows.hasPostmortem && !force) skipped += 1;
    for (const c of report.deathCauses)
      causeTotals.set(c.cause, (causeTotals.get(c.cause) ?? 0) + c.count);
    for (const st of report.stallSites)
      stallTotals.set(st.landmark, (stallTotals.get(st.landmark) ?? 0) + st.count);
    const deathTop = report.deathCauses[0];
    const stallTop = report.stallSites[0];
    indexRows.push(
      `| ${rows.id} | ${rows.mode} | ${report.outcome === 'win' ? '✓通关' : '未通关'} | ${report.attempts} | ${report.maxXCol} (${report.progressPct}%) | ${report.decisions} | ${(report.execRate * 100).toFixed(0)}% | ${report.vetoes} | ${report.p50LatencyMs} | ${deathTop ? `${deathTop.cause}×${deathTop.count}` : '-'} | ${stallTop ? `${stallTop.landmark}×${stallTop.count}` : '-'} |`,
    );
  }

  const rank = (m: Map<string, number>): string =>
    [...m.entries()]
      .sort((a, b) => b[1] - a[1])
      .map(([k, v]) => `${k}×${v}`)
      .join(', ') || '无';
  const index = [
    '# Laya Jump 会话索引(自动生成)',
    '',
    `目录:${dir} · 会话 ${indexRows.length} 场 · 复盘新生成 ${generated} 份${skipped > 0 ? `(已存在跳过 ${skipped},--force 重生成)` : ''}`,
    '',
    '| 会话 | 模式 | 结果 | 尝试 | 最远 col | 决策拍 | EXEC% | veto | p50ms | 死因Top | 停滞Top |',
    '|---|---|---|---|---|---|---|---|---|---|---|',
    ...indexRows,
    '',
    '## 跨会话聚合',
    `- 死因排行:${rank(causeTotals)}`,
    `- 停滞点排行:${rank(stallTotals)}`,
    '',
    '> 进化闭环:本表 → 云端编码代理改代码/参数/蒸馏 → 重编译 → A/B(≥10 局)→ 本表对比。',
    '',
  ].join('\n');
  fs.writeFileSync(path.join(dir, 'INDEX.md'), index, 'utf8');
  console.log(`✓ ${generated} 份复盘 → ${dir}`);
  console.log(
    index
      .split('\n')
      .slice(5, 5 + indexRows.length + 2)
      .join('\n'),
  );
}

main();
