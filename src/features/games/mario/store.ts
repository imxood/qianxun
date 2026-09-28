/**
 * 进化状态存储(docs/13 §2):Tauri IPC 为主,localStorage 兜底(纯 web dev)。
 * 本模块只搬运字节;内容的解析/校验/迁移在 evolution.ts(纯函数,可测)。
 * 无头闭环(e2e/mario-loop.ts)不走这里——它直接 fs 操作同一数据根。
 */

import { call } from '../../../lib/ipc';

export type MarioStore = {
  read(name: string): Promise<string | null>;
  write(name: string, content: string): Promise<void>;
  append(name: string, line: string): Promise<void>;
  list(): Promise<string[]>;
};

const LOCAL_PREFIX = 'qx-mario-state:';

/** 纯 web dev 兜底:无 Tauri 时用 localStorage 模拟同一组文件名。 */
function createLocalStore(): MarioStore {
  const safe = <T>(fn: () => T, fallback: T): T => {
    try {
      return fn();
    } catch {
      return fallback; // 隐私模式等:降级为内存语义,不阻断游戏
    }
  };
  return {
    read: (name) => Promise.resolve(safe(() => localStorage.getItem(LOCAL_PREFIX + name), null)),
    write: (name, content) => {
      safe(() => localStorage.setItem(LOCAL_PREFIX + name, content), undefined);
      return Promise.resolve();
    },
    append: (name, line) => {
      safe(() => {
        const prev = localStorage.getItem(LOCAL_PREFIX + name) ?? '';
        localStorage.setItem(
          LOCAL_PREFIX + name,
          prev + (line.endsWith('\n') ? line : line + '\n'),
        );
      }, undefined);
      return Promise.resolve();
    },
    list: () =>
      Promise.resolve(
        safe(() => {
          const out: string[] = [];
          for (let i = 0; i < localStorage.length; i += 1) {
            const key = localStorage.key(i);
            if (key?.startsWith(LOCAL_PREFIX)) out.push(key.slice(LOCAL_PREFIX.length));
          }
          return out.sort();
        }, []),
      ),
  };
}

function createIpcStore(): MarioStore {
  return {
    read: (name) => call<string | null>('mario_state_read', { name }),
    write: async (name, content) => {
      await call('mario_state_write', { name, content });
    },
    append: async (name, line) => {
      await call('mario_state_append', { name, line });
    },
    list: () => call<string[]>('mario_state_list'),
  };
}

function inTauri(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] === 'object'
  );
}

let cached: MarioStore | null = null;

/** 默认存储:Tauri 内走 IPC(数据根 mario/),纯 web dev 降级 localStorage。 */
export function getMarioStore(): MarioStore {
  cached ??= inTauri() ? createIpcStore() : createLocalStore();
  return cached;
}

/** 测试注入点。 */
export function setMarioStoreForTest(store: MarioStore | null): void {
  cached = store;
}
