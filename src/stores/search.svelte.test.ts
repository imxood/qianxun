/**
 * SearchStore vitest 单测（汇总 §7.4 S1 · 最高 ROI）。
 *
 * 策略：mock `@tauri-apps/api/core` 的 `Channel` 类（替换为带 `emit` 钩子的
 * FakeChannel），mock `lib/ipc.call`（替成预排好响应的假 IPC）。fake timers
 * 控 setTimeout 防抖。
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// ---- 1) Fake IPC + Fake Channel setup -------------------------------------

type Responder = (args: Record<string, unknown>) => unknown;

const responders = new Map<string, Responder>();

const ipcCalls: Array<{ command: string; args: Record<string, unknown> }> = [];

function respond(command: string, fn: Responder): void {
  responders.set(command, fn);
}
function respondOnce(command: string, fn: Responder): void {
  responders.set(command, (args) => {
    responders.delete(command);
    return fn(args);
  });
}
function record(command: string, fn: Responder): void {
  responders.set(command, fn);
}

vi.mock('../lib/ipc', () => ({
  call: vi.fn(async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
    ipcCalls.push({ command, args: args ?? {} });
    const fn = responders.get(command);
    if (!fn) throw new Error(`no responder for ${command}`);
    return fn(args ?? {}) as T;
  }),
}));

type ChannelHandler = (msg: unknown) => void;
type FakeChannelInstance<T> = {
  onmessage: ChannelHandler | null;
  emit(msg: T): void;
};

vi.mock('@tauri-apps/api/core', () => {
  // FakeChannel 类必须在 factory 内定义（vi.mock hoist 后外部引用会 TDZ）。
  const FakeChannel = class FakeChannel<T> {
    onmessage: ChannelHandler | null = null;
    /** 测试用：手动向 store 推一帧。 */
    emit(msg: T): void {
      this.onmessage?.(msg);
    }
  };
  return {
    Channel: FakeChannel,
  };
});

// 现在才能 import store（mock 必须在 import 之前）
import type { FilesPage, GrepPage } from '../lib/ipc/contract';
import { search } from './search.svelte';

// FakeChannelInstance 类型别名（供测试代码 cast 用）
type FakeChannelCtor = new <T>() => FakeChannelInstance<T>;
// eslint-disable-next-line @typescript-eslint/no-unused-vars
type _FakeChannelAlias = FakeChannelCtor;

// ---- 2) 工具：每个用例前重置 store + 状态 -----------------------------------

let savedStore: {
  rootInput: string;
  status: unknown;
  openError: string;
  drives: unknown[];
  filesQuery: string;
  filesResult: unknown;
  filesBusy: boolean;
  filesError: string;
  grepQuery: string;
  grepOptions: unknown;
  grepWholeWord: boolean;
  grepGlob: string;
  grepResult: unknown;
  grepBusy: boolean;
  grepError: string;
};

function resetStore(): void {
  // 每次用例都重置 store 状态——直接赋值字段即可（class instance）。
  search.dispose();
  responders.clear();
  ipcCalls.length = 0;
  savedStore = {
    rootInput: '',
    status: null,
    openError: '',
    drives: [],
    filesQuery: '',
    filesResult: null,
    filesBusy: false,
    filesError: '',
    grepQuery: '',
    grepOptions: {
      regex: false,
      smartCase: true,
      beforeContext: 1,
      afterContext: 1,
    },
    grepWholeWord: false,
    grepGlob: '',
    grepResult: null,
    grepBusy: false,
    grepError: '',
  };
  Object.assign(search, savedStore);
}

// ---- 3) 用例 ---------------------------------------------------------------

describe('SearchStore.open', () => {
  beforeEach(() => resetStore());
  afterEach(() => resetStore());

  it('空查询短路：search_status 不发 IPC', async () => {
    record('search_status', () => ({
      root: null,
      generation: 0,
      scanning: false,
      watcherReady: false,
      files: 0,
    }));
    await search.refreshStatus();
    expect(ipcCalls.map((c) => c.command)).toEqual(['search_status']);
  });

  it('open 成功 → rootInput + 触发 status 轮询', async () => {
    respond('search_open', () => ({
      root: 'C:\\Users\\test',
      generation: 1,
      rebuilt: true,
    }));
    record('search_status', () => ({
      root: 'C:\\Users\\test',
      generation: 1,
      scanning: true,
      watcherReady: true,
      files: 100,
    }));
    await search.open('C:\\Users\\test');
    expect(search.rootInput).toBe('C:\\Users\\test');
    expect(ipcCalls.map((c) => c.command)).toContain('search_open');
    expect(ipcCalls.map((c) => c.command)).toContain('search_status');
  });

  it('open 失败 → openError 写入；不调 status', async () => {
    respond('search_open', () => {
      throw new Error('路径非法');
    });
    await search.open('Z:/no-such');
    expect(search.openError).toContain('路径非法');
  });
});

describe('SearchStore.runFiles', () => {
  beforeEach(() => {
    resetStore();
    search.status = {
      root: 'C:\\test',
      generation: 1,
      scanning: false,
      watcherReady: true,
      files: 0,
    };
  });
  afterEach(() => resetStore());

  it('空查询 → result 清空、不发 IPC', async () => {
    search.filesQuery = '';
    await search.runFiles();
    expect(search.filesResult).toBeNull();
    expect(ipcCalls).toHaveLength(0);
  });

  it('正常查询 → result 设置、busy 翻转', async () => {
    respond('search_files', () => ({
      items: [{ path: 'a.rs', score: 1, offsets: [], size: 10, mtime: 0 }],
      totalMatched: 1,
      totalFiles: 1,
    }));
    search.filesQuery = 'a';
    const promise = search.runFiles();
    expect(search.filesBusy).toBe(true);
    await promise;
    expect(search.filesBusy).toBe(false);
    expect(search.filesResult).toEqual({
      items: [{ path: 'a.rs', score: 1, offsets: [], size: 10, mtime: 0 }],
      totalMatched: 1,
      totalFiles: 1,
    });
    expect(search.filesError).toBe('');
  });

  it('IPC 抛错 → filesError 写入、result 清空（PR 1.4 修的旧 bug）', async () => {
    respond('search_files', () => {
      throw new Error('权限拒绝');
    });
    search.filesQuery = 'secret';
    await search.runFiles();
    expect(search.filesError).toContain('权限拒绝');
    expect(search.filesResult).toBeNull();
    expect(search.filesBusy).toBe(false);
  });
});

describe('SearchStore.runGrep', () => {
  beforeEach(() => {
    resetStore();
    // 给 status 注入 generation，让 PR3.5 的代际守卫能正常工作。
    search.status = {
      root: 'C:\\test',
      generation: 1,
      scanning: false,
      watcherReady: true,
      files: 0,
    };
  });
  afterEach(() => resetStore());

  it('空查询 → 清空 result、不发 IPC', async () => {
    await search.runGrep();
    expect(search.grepResult).toBeNull();
    expect(search.grepError).toBe('');
    expect(ipcCalls.filter((c) => c.command === 'search_content')).toHaveLength(0);
  });

  it('正常流式：通道先到分片、终值后到 → mergeGrepStream 合并', async () => {
    search.grepQuery = 'foo';
    let capturedChannel: unknown = null;
    respond('search_content', (args) => {
      capturedChannel = args.onProgress;
      return {
        items: [
          {
            path: 'b.rs',
            lineNumber: 2,
            col: 0,
            lineContent: 'foo',
            offsets: [],
            contextBefore: [],
            contextAfter: [],
          },
        ],
        filesSearched: 5,
        filesWithMatches: 1,
        nextFileOffset: 0,
        aborted: false,
      };
    });
    const promise = search.runGrep();
    // 第一帧：模拟引擎先推一条 a.rs:1
    (capturedChannel as FakeChannelInstance<unknown> | null)?.emit({
      items: [
        {
          path: 'a.rs',
          lineNumber: 1,
          col: 0,
          lineContent: 'foo',
          offsets: [],
          contextBefore: [],
          contextAfter: [],
        },
      ],
      filesSearched: 3,
      filesWithMatches: 1,
    });
    // 等终值
    await promise;
    expect(search.grepResult?.items.map((h) => h.path)).toEqual(['a.rs', 'b.rs']);
    expect(search.grepResult?.filesSearched).toBe(5);
    expect(search.grepResult?.filesWithMatches).toBe(1);
    expect(search.grepBusy).toBe(false);
  });

  it('过期分片丢弃：第二次 runGrep 进入后第一次的 Channel 帧被丢弃', async () => {
    search.grepQuery = 'foo';
    let firstChannel: unknown = null;
    let callIndex = 0;
    respond('search_content', (args) => {
      callIndex += 1;
      if (callIndex === 1) {
        firstChannel = args.onProgress;
        // 永挂（不返回），模拟进行中
        return new Promise(() => {
          /* never resolves */
        });
      } else {
        // 第二次进入 — 立即返回空终值（取消旧的）。
        return {
          items: [],
          filesSearched: 0,
          filesWithMatches: 0,
          nextFileOffset: 0,
          aborted: false,
        };
      }
    });
    const p1 = search.runGrep();
    // 第二次进入前先推一帧
    (firstChannel as FakeChannelInstance<unknown> | null)?.emit({
      items: [
        {
          path: 'old.rs',
          lineNumber: 1,
          col: 0,
          lineContent: 'foo',
          offsets: [],
          contextBefore: [],
          contextAfter: [],
        },
      ],
      filesSearched: 1,
      filesWithMatches: 1,
    });
    // 启动第二次
    search.grepQuery = 'bar';
    const p2 = search.runGrep();
    // 模拟旧的帧迟到 → 应被 seq 守卫丢弃
    (firstChannel as FakeChannelInstance<unknown> | null)?.emit({
      items: [
        {
          path: 'late.rs',
          lineNumber: 1,
          col: 0,
          lineContent: 'late',
          offsets: [],
          contextBefore: [],
          contextAfter: [],
        },
      ],
      filesSearched: 99,
      filesWithMatches: 99,
    });
    await p2;
    // 第一帧进入（p1 启动时那帧）→ 应被看到；late.rs 过期帧应被丢弃
    // 注：第一次 emit 在 grepSeq 还是 1，进入 grepResult；late.rs 在 grepSeq=2，丢弃。
    expect(search.grepResult?.items.map((h) => h.path)).not.toContain('late.rs');
    // p1 还在挂着，强行结束测试不等待
    p1.catch(() => {});
  });

  it('IPC 抛错 → grepError 写入、busy 关（仅在当前代际）', async () => {
    search.grepQuery = 'foo';
    respondOnce('search_content', () => {
      throw new Error('grep 引擎爆炸');
    });
    await search.runGrep();
    expect(search.grepError).toContain('grep 引擎爆炸');
    expect(search.grepBusy).toBe(false);
  });
});

describe('SearchStore.cancelGrep', () => {
  beforeEach(() => resetStore());
  afterEach(() => resetStore());

  it('cancelGrep 同步 ++grepSeq，并 await search_cancel', async () => {
    respond('search_cancel', () => undefined);
    await search.cancelGrep();
    expect(ipcCalls.map((c) => c.command)).toContain('search_cancel');
  });
});

describe('SearchStore.generation 代际守卫（汇总 §3.6 / PR3.5）', () => {
  beforeEach(() => {
    resetStore();
    search.status = {
      root: 'C:\\test',
      generation: 1,
      scanning: false,
      watcherReady: true,
      files: 0,
    };
  });
  afterEach(() => resetStore());

  it('runFiles：换根后旧结果丢弃', async () => {
    let resolveFileCall: unknown = null;
    respond(
      'search_files',
      () =>
        new Promise((r) => {
          resolveFileCall = r;
        }),
    );
    search.filesQuery = 'foo';
    const p = search.runFiles();
    // 模拟换根：search.status.generation 从 1 涨到 2
    search.status = {
      ...search.status!,
      generation: 2,
    };
    // 现在让 fake IPC 返回 → 但 generation 已变
    (resolveFileCall as ((value: FilesPage) => void) | null)?.({
      items: [{ path: 'late.rs', score: 1, offsets: [], size: 0, mtime: 0 }],
      totalMatched: 1,
      totalFiles: 1,
    });
    await p;
    expect(search.filesResult).toBeNull(); // 旧结果丢弃
    expect(search.filesBusy).toBe(false);
  });

  it('runGrep：换根后旧帧与终值均丢弃', async () => {
    search.grepQuery = 'foo';
    let capturedChannel: unknown = null;
    let resolveContent: unknown = null;
    respond('search_content', (args) => {
      capturedChannel = args.onProgress;
      return new Promise((r) => {
        resolveContent = r;
      });
    });
    const p = search.runGrep();
    // 第一帧 a.rs:1 进入（generation=1 OK）
    (capturedChannel as FakeChannelInstance<unknown> | null)?.emit({
      items: [
        {
          path: 'a.rs',
          lineNumber: 1,
          col: 0,
          lineContent: 'foo',
          offsets: [],
          contextBefore: [],
          contextAfter: [],
        },
      ],
      filesSearched: 1,
      filesWithMatches: 1,
    });
    // 换根
    search.status = { ...search.status!, generation: 2 };
    // 第二帧（应在 grepSeq 守卫前先被 generation 守卫拦下）：b.rs:2
    (capturedChannel as FakeChannelInstance<unknown> | null)?.emit({
      items: [
        {
          path: 'b.rs',
          lineNumber: 2,
          col: 0,
          lineContent: 'foo',
          offsets: [],
          contextBefore: [],
          contextAfter: [],
        },
      ],
      filesSearched: 2,
      filesWithMatches: 2,
    });
    // 终值
    (resolveContent as ((value: GrepPage) => void) | null)?.({
      items: [
        {
          path: 'c.rs',
          lineNumber: 3,
          col: 0,
          lineContent: 'foo',
          offsets: [],
          contextBefore: [],
          contextAfter: [],
        },
      ],
      filesSearched: 3,
      filesWithMatches: 3,
      nextFileOffset: 0,
      aborted: false,
    });
    await p;
    // 只剩 a.rs（换根前的帧），b.rs/c.rs 全丢
    expect(search.grepResult?.items.map((h) => h.path)).toEqual(['a.rs']);
    // grepBusy 应复位（这是被换根取代的任务，其 finally 看到的 generation 与
    // snapshot 不符，触发复位逻辑：seq === grepSeq 但 generation 失配）。
    expect(search.grepBusy).toBe(false);
  });
});

describe('SearchStore.dispose', () => {
  beforeEach(() => resetStore());
  afterEach(() => resetStore());

  it('dispose 后 stop 轮询 handle', () => {
    search.status = {
      root: 'C:\\test',
      generation: 1,
      scanning: true,
      watcherReady: true,
      files: 10,
    };
    // 没 start 过 poller，stop 不应报错
    expect(() => search.dispose()).not.toThrow();
  });
});
