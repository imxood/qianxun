import { describe, expect, it } from 'vitest';
import type { DiskEntry, DiskHome, DiskScanProgress } from '../../../lib/ipc/contract';
import {
  applyLiveGrow,
  isManaged,
  normPath,
  pathTail,
  provisionalItem,
  type DiskTrailItem,
} from './scanner.svelte';

function entry(
  name: string,
  path: string,
  size: number,
  dir = true,
  children: DiskEntry[] = [],
): DiskEntry {
  return { name, path, size, dir, children };
}

function trailItem(rootEntry: DiskEntry, at = Date.now()): DiskTrailItem {
  return {
    entry: rootEntry,
    at,
    partial: null,
    skipped: 0,
    largest: [],
  };
}

function frame(
  root: string,
  bytes: number,
  topChildren: Array<{ name: string; path: string; size: number; dir: boolean }>,
): DiskScanProgress {
  return {
    type: 'progress',
    root,
    bytes,
    files: topChildren.length,
    dirs: topChildren.filter((c) => c.dir).length,
    topChildren,
    skipped: 0,
  };
}

describe('normPath', () => {
  it('去掉尾部反斜杠/正斜杠', () => {
    expect(normPath('C:\\Users\\')).toBe('c:\\users');
    expect(normPath('C:/Users/')).toBe('c:/users');
    expect(normPath('C:\\Users\\')).toBe('c:\\users');
  });
  it('Windows 路径转小写', () => {
    expect(normPath('C:\\Users\\MAXU')).toBe('c:\\users\\maxu');
  });
});

describe('pathTail', () => {
  it('取最末一级', () => {
    expect(pathTail('C:\\Users\\maxu\\Documents')).toBe('Documents');
    expect(pathTail('C:/Users/maxu/Documents/')).toBe('Documents');
  });
  it('空字符串兜底为原值', () => {
    expect(pathTail('/')).toBe('');
  });
});

describe('isManaged', () => {
  const home: DiskHome = {
    root: 'C:\\Users\\maxu\\AppData',
    labels: { qianxun: '千寻', dsh: 'DSH' },
  };

  it('home=null 时全 false', () => {
    expect(isManaged(entry('qianxun', 'C:\\Users\\maxu\\AppData\\qianxun', 0), null)).toBe(false);
  });

  it('数据根直接子项里的有 label 的目录 → true', () => {
    expect(isManaged(entry('qianxun', 'C:\\Users\\maxu\\AppData\\qianxun', 0), home)).toBe(true);
    expect(isManaged(entry('dsh', 'C:\\Users\\maxu\\AppData\\dsh', 0), home)).toBe(true);
  });

  it('数据根直接子项里没 label 的目录 → false', () => {
    expect(isManaged(entry('random', 'C:\\Users\\maxu\\AppData\\random', 0), home)).toBe(false);
  });

  it('非数据根直接子项 → false（即便名字与 label 同名）', () => {
    expect(isManaged(entry('qianxun', 'C:\\Other\\AppData\\qianxun', 0), home)).toBe(false);
  });

  it('大小写不敏感：路径大小写差异不影响判定', () => {
    // home.labels 字典 key 大小写敏感；数据根路径经 normPath 归一化大小写，
    // 因此只要 entry.name 与 labels key 大小写一致 + 父目录归一化匹配即可。
    expect(isManaged(entry('qianxun', 'C:\\USERS\\MAXU\\APPDATA\\qianxun', 0), home)).toBe(true);
  });
});

describe('provisionalItem', () => {
  it('返回 size=0 的占位视图条目', () => {
    const item = provisionalItem('C:\\Users\\maxu');
    expect(item.entry.name).toBe('maxu');
    expect(item.entry.size).toBe(0);
    expect(item.entry.children).toEqual([]);
    expect(item.at).toBe(0);
    expect(item.partial).toBeNull();
  });

  it('空路径兜底为原 path', () => {
    const item = provisionalItem('');
    expect(item.entry.path).toBe('');
    expect(item.entry.name).toBe('');
  });
});

describe('applyLiveGrow', () => {
  it('空 trail：changed=false', () => {
    const result = applyLiveGrow(frame('C:\\root', 100, []), [], 0);
    expect(result.changed).toBe(false);
    expect(result.nextTrail).toEqual([]);
  });

  it('trail 末项路径不匹配 → 不变（防止串数据）', () => {
    const trail = [trailItem(entry('qianxun', 'C:\\Users\\maxu\\AppData\\qianxun', 1000))];
    const result = applyLiveGrow(
      frame('D:\\other', 100, [{ name: 'a', path: 'D:\\other\\a', size: 50, dir: true }]),
      trail,
      0,
    );
    expect(result.changed).toBe(false);
    expect(result.nextTrail).toBe(trail);
  });

  it('first frame：合并子项到占位视图', () => {
    const trail = [provisionalItem('C:\\Users\\maxu')];
    const result = applyLiveGrow(
      frame('C:\\Users\\maxu', 1024, [
        { name: 'a', path: 'C:\\Users\\maxu\\a', size: 512, dir: true },
        { name: 'b', path: 'C:\\Users\\maxu\\b', size: 512, dir: false },
      ]),
      trail,
      0,
    );
    expect(result.changed).toBe(true);
    expect(result.nextTrail).toHaveLength(1);
    const top = result.nextTrail[0]!.entry;
    expect(top.children).toHaveLength(2);
    expect(top.size).toBe(1024);
  });

  it('复用 entry 对象（路径不变时改 size）', () => {
    // trail 末项的 path 必须等于 frame.root，否则 liveGrow 跳过（见前测）。
    // 注意：nextChildren 从 frame.topChildren 重建；旧 children 中不在本帧的会被丢
    // 掉（这是边扫边长特性：每帧后端推它"已知"的子项）。
    const rootPath = 'C:\\Users\\maxu\\AppData\\qianxun';
    const firstChildren = [
      entry('a', `${rootPath}\\a`, 100, true),
      entry('b', `${rootPath}\\b`, 200, false),
    ];
    const trail = [trailItem(entry('qianxun', rootPath, 1000, true, firstChildren))];
    const result = applyLiveGrow(
      frame(rootPath, 1000, [{ name: 'a', path: `${rootPath}\\a`, size: 999, dir: true }]),
      trail,
      0,
    );
    expect(result.changed).toBe(true);
    const top = result.nextTrail[0]!.entry;
    // a 的 size 应被更新（path 一致）→ 原 entry 对象被复用，引用稳定
    const a = top.children.find((c) => c.name === 'a');
    expect(a?.size).toBe(999);
    // top.size 来自 frame.bytes
    expect(top.size).toBe(1000);
  });

  it('契约防御：topChildren 不是数组时不变（docs/07 §2.3）', () => {
    const trail = [provisionalItem('C:\\Users\\maxu')];
    // frame 故意把 topChildren 置为 null（模拟后端字段漂移）
    const badFrame = {
      type: 'progress' as const,
      root: 'C:\\Users\\maxu',
      bytes: 100,
      files: 0,
      dirs: 0,
      topChildren: null as unknown as Array<{
        name: string;
        path: string;
        size: number;
        dir: boolean;
      }>,
      skipped: 0,
    };
    const result = applyLiveGrow(badFrame, trail, 0);
    expect(result.changed).toBe(false);
    expect(result.nextTrail).toBe(trail);
  });

  it('节流：100ms 内的连续帧被吞掉', () => {
    const trail = [provisionalItem('C:\\root')];
    // 第一次更新 lastLive 到当前时间
    const lastLive = Date.now();
    // 立即再发一帧（间隔 < 100ms）应被节流
    const result = applyLiveGrow(
      frame('C:\\root', 200, [{ name: 'a', path: 'C:\\root\\a', size: 100, dir: true }]),
      trail,
      lastLive,
    );
    expect(result.changed).toBe(false);
  });
});
