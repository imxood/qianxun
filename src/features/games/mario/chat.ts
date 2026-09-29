/**
 * 聊天式进化台的消息模型与纯函数(docs/16 §2/§4)。
 * 只做数据:类型、upsert、审计降级重建——全部可单测;UI 在 EvolutionChat.svelte。
 * 5 种元素:局分隔符 / 死亡行 / 分析卡(占位→结论原地变身)/ 提案卡 / 判定行。
 */

export type ChatPatch = Record<string, { from: unknown; to: unknown }>;

export type ChatMsg = {
  id: string;
  kind: 'sep' | 'death' | 'analysis' | 'proposal' | 'verdict';
  runId: string;
  ts: number;
  /** 局分隔符:live=进行中,局末落 result。 */
  result?: 'live' | 'lose' | 'win';
  iter?: number;
  mode?: string;
  deaths?: number;
  maxXCol?: number;
  score?: number;
  /** 死亡行 / 分析卡:attempt(run 级为 null) */
  attempt?: number | null;
  cause?: string;
  col?: number;
  pct?: number;
  /** 分析卡:key=runId#attempt;phase queued→running→ok/fail 原地变身 */
  key?: string;
  reason?: string;
  phase?: 'queued' | 'running' | 'ok' | 'fail';
  latencyMs?: number;
  /** 结论卡正文(diagnosis 要点,≤2 句)+ 元数据行 */
  text?: string;
  meta?: string[];
  issues?: string[];
  /** 提案卡 */
  patch?: ChatPatch;
  patchKind?: 'numeric' | 'design';
  gated?: boolean;
  verdict?: 'commit' | 'rollback' | 'restore' | null;
  /** 判定行 */
  ok?: boolean;
};

let seq = 0;

/** 消息真 id:`runId#seq`(docs/16 §4.3,禁下标/时间戳拼接)。 */
export function chatMsgId(runId: string): string {
  seq += 1;
  return `${runId}#${seq}`;
}

/** 上限:内存滚动窗(docs/16 §4.2,事实在 chat.jsonl/审计,丢了可重建)。 */
export const CHAT_CAP = 199;

/** 追加一条(滚动窗)。 */
export function chatAppend(list: ChatMsg[], m: ChatMsg): ChatMsg[] {
  return [...list.slice(-CHAT_CAP), m];
}

/** 按 id 原地更新(分析卡变身/sep 落定/提案判定角标)。找不到不动。 */
export function chatPatch(list: ChatMsg[], id: string, patch: Partial<ChatMsg>): ChatMsg[] {
  let hit = false;
  const out = list.map((m) => {
    if (m.id !== id) return m;
    hit = true;
    return { ...m, ...patch };
  });
  return hit ? out : list;
}

/** 按 key 找分析卡 id(占位→变身)。 */
export function chatFindAnalysis(list: ChatMsg[], key: string): ChatMsg | null {
  return list.find((m) => m.kind === 'analysis' && m.key === key) ?? null;
}

export type AuditLike = {
  ts?: string;
  iter?: number;
  runId?: string | null;
  action?: string;
  phase?: string;
  key?: string;
  reason?: string;
  latencyMs?: number;
  errClass?: string;
  patch?: ChatPatch;
  patchKind?: 'numeric' | 'design';
  score?: { champion?: number | null; candidate?: number | null };
};

/**
 * 审计降级重建(docs/16 §5 方案 B):chat.jsonl 缺失/损坏时,从 evolution.jsonl
 * 尾部重建"简史"——只有事件骨架,无 Qwen 结论原文(UI 标注"历史由审计重建")。
 */
export function rebuildFromAudit(audits: AuditLike[]): ChatMsg[] {
  const out: ChatMsg[] = [];
  for (const a of audits) {
    if (typeof a.runId !== 'string' || a.runId === '') continue;
    const ts = a.ts ? Date.parse(a.ts) || 0 : 0;
    const id = chatMsgId(a.runId);
    if (a.action === 'analyze' && typeof a.key === 'string') {
      const phase =
        a.phase === 'ok'
          ? 'ok'
          : a.phase === 'fail'
            ? 'fail'
            : a.phase === 'start'
              ? 'running'
              : 'queued';
      out.push({
        id,
        kind: 'analysis',
        runId: a.runId,
        ts,
        key: a.key,
        attempt: Number(a.key.split('#')[1] ?? 0) || null,
        reason: a.reason ?? '',
        phase,
        latencyMs: a.latencyMs,
        ...(a.phase === 'fail' ? { text: a.errClass === 'timeout' ? '超时' : '失败' } : {}),
      });
    } else if (a.action === 'propose' && a.patch) {
      out.push({
        id,
        kind: 'proposal',
        runId: a.runId,
        ts,
        patch: a.patch,
        patchKind: a.patchKind ?? 'numeric',
        gated: false,
      });
    } else if (a.action === 'commit' || a.action === 'rollback' || a.action === 'restore') {
      const s = a.score;
      const note =
        s?.champion != null && s?.candidate != null
          ? `${s.candidate.toFixed(0)} vs ${s.champion.toFixed(0)}`
          : (a.reason ?? '');
      out.push({
        id,
        kind: 'verdict',
        runId: a.runId,
        ts,
        ok: a.action === 'commit' || a.action === 'restore',
        text: `${a.action === 'commit' ? '转正' : a.action === 'restore' ? '回退' : '回滚'} · ${note}`,
      });
    }
  }
  return out;
}

/** 解析 chat.jsonl 内容;坏行跳过,非 5 类元素丢弃。 */
export function parseChat(raw: string | null): ChatMsg[] {
  if (raw === null) return [];
  const out: ChatMsg[] = [];
  for (const line of raw.split('\n')) {
    if (line.trim() === '') continue;
    try {
      const m = JSON.parse(line) as ChatMsg;
      if (
        typeof m === 'object' &&
        m !== null &&
        typeof m.id === 'string' &&
        typeof m.runId === 'string' &&
        ['sep', 'death', 'analysis', 'proposal', 'verdict'].includes(m.kind)
      ) {
        out.push(m);
      }
    } catch {
      // 坏行跳过:呈现层宁缺毋崩
    }
  }
  return out;
}

/** 按 runId 顺序分组(时间线即插入序;键为 runId)。 */
export function chatGroups(list: ChatMsg[]): Array<{ runId: string; msgs: ChatMsg[] }> {
  const groups: Array<{ runId: string; msgs: ChatMsg[] }> = [];
  const index = new Map<string, number>();
  for (const m of list) {
    let i = index.get(m.runId);
    if (i === undefined) {
      i = groups.length;
      index.set(m.runId, i);
      groups.push({ runId: m.runId, msgs: [] });
    }
    groups[i]!.msgs.push(m);
  }
  return groups;
}
