/**
 * 流式 grep 分片合并工具（汇总 §3.1 / 04 §S1）：
 *
 * 后端 `commands.rs::search_content` 流式推送 `GrepProgress.items`（每分片
 * ≤100 条），最后再 await `GrepPage` 终值。旧实现按 `page.items.length > items.length`
 * 比较两端长短后选大的——这是错误的：后端终值可能去重（如同一 (path,lineNumber)
 * 只留一条），长度关系不可靠；通道帧偶发空帧也会把累计长度弄错。
 *
 * 正确语义：以 `(path, lineNumber)` 为键做集合并，**始终信任通道累计
 * 与终值的并集**（覆盖最广）；终值额外带 `nextFileOffset` / `aborted` /
 * `filesSearched` / `filesWithMatches` 这四个聚合字段（一次性返回，通道
 * 不发），取终值的。
 */

import type { GrepHit, GrepPage, GrepProgress } from '../../lib/ipc/contract';

export interface MergeResult {
  items: GrepHit[];
  filesSearched: number;
  filesWithMatches: number;
  nextFileOffset: number;
  aborted: boolean;
}

/**
 * 把通道累计的 GrepProgress[] 与后端一次性返回的 GrepPage 终值合并。
 *
 * - `items` 按 (path, lineNumber) 去重并集；同 key 的 hit 优先取累计里较晚
 *   的（通道帧按时间序到达，引擎增量去重后端的命中顺序与通道一致）。
 * - `filesSearched` / `filesWithMatches` 取较大值（防御：终值可能略晚到）。
 * - `nextFileOffset` / `aborted` 取终值（通道不报这两字段）。
 *
 * 任一端为 `undefined` 时按空集合处理；终值缺失时退化为通道累计 + 全 0 聚合。
 */
export function mergeGrepStream(chunks: GrepProgress[], final: GrepPage | undefined): MergeResult {
  const dedupe = new Map<string, GrepHit>();
  let filesSearched = 0;
  let filesWithMatches = 0;

  for (const chunk of chunks) {
    for (const hit of chunk.items) {
      dedupe.set(keyOf(hit), hit);
    }
    filesSearched = Math.max(filesSearched, chunk.filesSearched);
    filesWithMatches = Math.max(filesWithMatches, chunk.filesWithMatches);
  }

  if (final) {
    for (const hit of final.items) {
      dedupe.set(keyOf(hit), hit);
    }
    filesSearched = Math.max(filesSearched, final.filesSearched);
    filesWithMatches = Math.max(filesWithMatches, final.filesWithMatches);
  }

  return {
    items: [...dedupe.values()],
    filesSearched,
    filesWithMatches,
    nextFileOffset: final?.nextFileOffset ?? 0,
    aborted: final?.aborted ?? false,
  };
}

function keyOf(hit: GrepHit): string {
  // col 参与去重：同一行多次匹配（如 grep -o 风格）若 col 相同即为同一命中；
  // 不同 col 是不同命中（如 `foo bar foo` 搜 `foo` 命中 0 列 + 8 列）应都保留。
  // offsets 也参与：避免引擎偶发重复推送（理论上不会，但稳一点）。
  return `${hit.path}\u0001${hit.lineNumber}\u0001${hit.col}\u0001${hit.offsets.length}`;
}
