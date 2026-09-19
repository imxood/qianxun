import { describe, expect, it } from 'vitest';
import type { GrepHit, GrepPage, GrepProgress } from '../../lib/ipc/contract';
import { mergeGrepStream } from './grep-merge';

function hit(path: string, line: number, col = 0, content = ''): GrepHit {
  return {
    path,
    lineNumber: line,
    col,
    lineContent: content,
    offsets: [[0, 1]],
    contextBefore: [],
    contextAfter: [],
  };
}

function chunk(items: GrepHit[], filesSearched = 0, filesWithMatches = 0): GrepProgress {
  return { items, filesSearched, filesWithMatches };
}

function final(items: GrepHit[], partial: Partial<GrepPage> = {}): GrepPage {
  return {
    items,
    filesSearched: 0,
    filesWithMatches: 0,
    nextFileOffset: 0,
    aborted: false,
    ...partial,
  };
}

describe('mergeGrepStream', () => {
  it('空输入 → 空结果', () => {
    expect(mergeGrepStream([], undefined)).toEqual({
      items: [],
      filesSearched: 0,
      filesWithMatches: 0,
      nextFileOffset: 0,
      aborted: false,
    });
  });

  it('只有通道累计、无 final → 聚合字段取累计 max', () => {
    const result = mergeGrepStream(
      [chunk([hit('a.rs', 1)], 5, 1), chunk([hit('b.rs', 2)], 10, 2)],
      undefined,
    );
    expect(result.items).toHaveLength(2);
    expect(result.filesSearched).toBe(10);
    expect(result.filesWithMatches).toBe(2);
    expect(result.nextFileOffset).toBe(0);
    expect(result.aborted).toBe(false);
  });

  it('只有 final、无通道累计 → 直接返回 final items', () => {
    const result = mergeGrepStream([], final([hit('a.rs', 1)], { filesSearched: 3 }));
    expect(result.items).toHaveLength(1);
    expect(result.filesSearched).toBe(3);
  });

  it('通道与 final 都有 → 按 (path,lineNumber) 去重并集', () => {
    const result = mergeGrepStream(
      [chunk([hit('a.rs', 1), hit('a.rs', 2)])],
      final([hit('a.rs', 2), hit('a.rs', 3), hit('a.rs', 4)]),
    );
    // a.rs:1,2,3,4 全保留（1/2 来自通道；3/4 仅 final 出现）
    expect(result.items.map((h) => `${h.path}:${h.lineNumber}`)).toEqual([
      'a.rs:1',
      'a.rs:2',
      'a.rs:3',
      'a.rs:4',
    ]);
  });

  it('同一 (path,lineNumber) 多 col → 全部保留（不同 col 是不同命中）', () => {
    const result = mergeGrepStream(
      [],
      final([hit('a.rs', 1, 0), hit('a.rs', 1, 5), hit('a.rs', 1, 10)]),
    );
    expect(result.items).toHaveLength(3);
  });

  it('完全相同命中重复 → 只保留一个', () => {
    const h = hit('a.rs', 1, 0, 'foo');
    const result = mergeGrepStream([], final([h, h, h]));
    expect(result.items).toHaveLength(1);
  });

  it('聚合字段：取 max（防御 final 略晚到）', () => {
    const result = mergeGrepStream(
      [chunk([hit('a.rs', 1)], 100, 10)],
      final([hit('a.rs', 1)], { filesSearched: 50, filesWithMatches: 5 }),
    );
    expect(result.filesSearched).toBe(100);
    expect(result.filesWithMatches).toBe(10);
  });

  it('nextFileOffset / aborted 仅取 final（通道不发）', () => {
    const result = mergeGrepStream(
      [chunk([hit('a.rs', 1)], 1, 1)],
      final([hit('a.rs', 1)], { nextFileOffset: 42, aborted: true }),
    );
    expect(result.nextFileOffset).toBe(42);
    expect(result.aborted).toBe(true);
  });

  it('通道累计 items 多、final items 少 → 仍取并集（修旧 length 比较 bug）', () => {
    // 旧实现 `page.items.length > items.length ? page.items : items` 会丢
    // 掉 final 里的独有命中（如引擎 dedup 后 final 更短）。
    const result = mergeGrepStream(
      [chunk([hit('a.rs', 1), hit('a.rs', 2), hit('a.rs', 3)], 5, 3)],
      final([hit('a.rs', 2)]),
    );
    expect(result.items.map((h) => h.lineNumber)).toEqual([1, 2, 3]);
  });
});
