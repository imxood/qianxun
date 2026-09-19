/**
 * 磁盘扫描纯逻辑层（汇总 §3.4 / PR2 治本 A · 拆分第一阶段）。
 *
 * 原 DiskScan.svelte 1448 行单文件，本模块抽出**纯函数**（无 IPC、无 state
 * 副作用）方便单测覆盖：
 * - 路径归一化 / 路径尾段
 * - 占位视图构造
 * - 受管目录检测
 * - 边扫边长（liveGrow）算法
 *
 * Channel 流式扫描 + 状态机仍留在 DiskScan.svelte，但通过本模块提供的
 * `applyLiveGrow` 复用边扫边长算法，确保契约一致（docs/07 §2.3 的
 * `Array.isArray` 守门集中在一处）。
 *
 * **完整拆分**（scanner 自持 $state + Channel 编排）放后续 PR；本次只
 * 抽算法层，避免单次大爆炸引入视觉回归。
 */

import type {
  DiskEntry,
  DiskHome,
  DiskLargeFile,
  DiskPartialChild,
  DiskScanProgress,
} from '../../../lib/ipc/contract';

export interface DiskTrailItem {
  entry: DiskEntry;
  at: number;
  partial: { files: number; dirs: number } | null;
  skipped: number;
  largest: DiskLargeFile[];
}

// ---- 路径工具 --------------------------------------------------------------

/** Windows 大小写不敏感 + 去掉尾部分隔符。 */
export function normPath(path: string): string {
  return path.replace(/[\\/]+$/, '').toLowerCase();
}

/** 取路径的尾段（最末一级名）。 */
export function pathTail(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '');
  return trimmed.split(/[\\/]/).pop() ?? trimmed;
}

/** 受管目录：数据根直接子项里被千寻起过友好名的（清理会破坏功能）。 */
export function isManaged(entry: DiskEntry, home: DiskHome | null): boolean {
  if (!home) return false;
  const trimmed = entry.path.replace(/[\\/]+$/, '');
  const parent = trimmed.replace(/[\\/][^\\/]*$/, '');
  return parent.length > 0 && normPath(parent) === normPath(home.root) && entry.name in home.labels;
}

// ---- 流式扫描边扫边长算法 -------------------------------------------------

/** 占位视图条目（扫描启动时 trail 的末项，size=0、at=0 表示占位）。 */
export function provisionalItem(path: string): DiskTrailItem {
  return {
    entry: { name: pathTail(path) || path, path, size: 0, dir: true, children: [] },
    at: 0,
    partial: null,
    skipped: 0,
    largest: [],
  };
}

function toEntry(child: DiskPartialChild): DiskEntry {
  return { name: child.name, path: child.path, size: child.size, dir: child.dir, children: [] };
}

export interface LiveGrowResult {
  /** 替换 trail 末项后的新 trail；无变化时等同传入。 */
  nextTrail: DiskTrailItem[];
  /** 更新后的 lastLive（节流时间戳）。 */
  nextLastLive: number;
  /** 是否实际有变化（false = 节流或契约防御）。 */
  changed: boolean;
}

/**
 * 边扫边长：进度帧的「根直接子项部分占用」按 path 合并进 trail 末项。
 * 复用 entry 对象、整组替换 trail 末项（确保 Svelte 5 $derived 触发）。
 *
 * 集中契约防御（docs/07 §2.3）：topChildren 缺失时只放弃本帧，绝不抛错。
 *
 * 返回 LiveGrowResult；调用方负责把 nextTrail 写回 state。
 */
export function applyLiveGrow(
  frame: DiskScanProgress,
  trail: DiskTrailItem[],
  lastLive: number,
): LiveGrowResult {
  const top = trail[trail.length - 1];
  if (!top || normPath(top.entry.path) !== normPath(frame.root)) {
    return { nextTrail: trail, nextLastLive: lastLive, changed: false };
  }
  const stamp = Date.now();
  if (stamp - lastLive < 100) {
    return { nextTrail: trail, nextLastLive: lastLive, changed: false };
  }
  if (!Array.isArray(frame.topChildren)) {
    return { nextTrail: trail, nextLastLive: lastLive, changed: false };
  }
  // 复用旧 entry 对象：原位改 size 不会触发深 proxy 失效，但
  // 替换整组 trail 末项会——这条路径是确定触发 $derived 重算的关键。
  const existing: Record<string, DiskEntry> = {};
  for (const child of top.entry.children) existing[child.path] = child;
  for (const partial of frame.topChildren) {
    const prev = existing[partial.path];
    if (prev) prev.size = partial.size;
  }
  // 即便 backend 给的 topChildren 与上一帧同序同 size 也要换数组引用，
  // 否则 $derived 看不到引用变化、settled 之后界面就会冻在旧状态。
  const nextChildren = frame.topChildren.map(
    (partial) => existing[partial.path] ?? toEntry(partial),
  );
  const nextEntry: DiskEntry = { ...top.entry, children: nextChildren, size: frame.bytes };
  return {
    nextTrail: [...trail.slice(0, -1), { ...top, entry: nextEntry }],
    nextLastLive: stamp,
    changed: true,
  };
}
