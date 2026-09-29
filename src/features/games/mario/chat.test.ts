import { describe, expect, it } from 'vitest';
import {
  chatAppend,
  chatFindAnalysis,
  chatGroups,
  chatMsgId,
  chatPatch,
  parseChat,
  rebuildFromAudit,
  type ChatMsg,
} from './chat';

const sep = (runId: string, over: Partial<ChatMsg> = {}): ChatMsg => ({
  id: chatMsgId(runId),
  kind: 'sep',
  runId,
  ts: 1,
  result: 'live',
  ...over,
});
const ana = (runId: string, key: string, over: Partial<ChatMsg> = {}): ChatMsg => ({
  id: chatMsgId(runId),
  kind: 'analysis',
  runId,
  ts: 1,
  key,
  attempt: 1,
  reason: 'death(pit@gap)',
  phase: 'queued',
  ...over,
});

describe('chat.ts 消息模型(docs/16 §4)', () => {
  it('id 真键单调;append 滚动窗 cap', () => {
    const a = chatMsgId('r1');
    const b = chatMsgId('r1');
    expect(a).not.toBe(b);
    expect(a.startsWith('r1#')).toBe(true);
    let list: ChatMsg[] = [];
    for (let i = 0; i < 300; i += 1) list = chatAppend(list, sep('r', { id: `x${i}` }));
    expect(list.length).toBeLessThanOrEqual(200);
    expect(list.at(-1)?.id).toBe('x299');
  });

  it('分析卡原地变身:chatPatch 按 id 命中,找不到不动', () => {
    let list = chatAppend([], ana('r1', 'r1#1'));
    const id = chatFindAnalysis(list, 'r1#1')!.id;
    list = chatPatch(list, id, { phase: 'running' });
    expect(chatFindAnalysis(list, 'r1#1')?.phase).toBe('running');
    list = chatPatch(list, id, { phase: 'ok', text: '结论', latencyMs: 9000 });
    const done = chatFindAnalysis(list, 'r1#1')!;
    expect(done.phase).toBe('ok');
    expect(done.text).toBe('结论');
    expect(chatPatch(list, '不存在', { phase: 'ok' })).toBe(list); // 未命中返回原数组
  });

  it('分组:按 runId 保序聚组', () => {
    const list = [
      ...chatAppend([], sep('r1')),
      ...chatAppend([], sep('r2')),
      ...chatAppend([], ana('r1', 'r1#1')),
    ];
    const groups = chatGroups(list);
    expect(groups.map((g) => g.runId)).toEqual(['r1', 'r2']);
    expect(groups[0]!.msgs).toHaveLength(2);
  });

  it('chat.jsonl 解析容错:坏行/外来 kind 丢弃', () => {
    const good = sep('r1', { result: 'lose', deaths: 3, maxXCol: 78, iter: 5, mode: 'reflex' });
    const raw = `${JSON.stringify(good)}\n{bad}\n${JSON.stringify({ kind: 'hack', id: 'x' })}\n`;
    const out = parseChat(raw);
    expect(out).toHaveLength(1);
    expect(out[0]!.kind).toBe('sep');
    expect(parseChat(null)).toEqual([]);
  });

  it('审计降级重建:analysis/propose/verdict 骨架,无原文', () => {
    const msgs = rebuildFromAudit([
      { runId: 'r1', action: 'analyze', phase: 'start', key: 'r1#1', reason: 'death(pit@x)' },
      { runId: 'r1', action: 'analyze', phase: 'ok', key: 'r1#1', latencyMs: 9000 },
      { runId: 'r1', action: 'propose', patch: { a: { from: 1, to: 2 } }, patchKind: 'numeric' },
      { runId: 'r1', action: 'rollback', score: { champion: 100, candidate: 101 } },
      { runId: null, action: 'commit' },
    ]);
    expect(msgs).toHaveLength(4);
    expect(msgs.filter((m) => m.kind === 'analysis')).toHaveLength(2);
    expect(msgs.find((m) => m.kind === 'proposal')?.patchKind).toBe('numeric');
    expect(msgs.find((m) => m.kind === 'verdict')?.ok).toBe(false);
    expect(msgs.every((m) => m.runId === 'r1')).toBe(true);
  });
});
