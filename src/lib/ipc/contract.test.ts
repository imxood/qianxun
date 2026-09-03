import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { IPC_COMMANDS } from './contract';

/**
 * 合同比对测试（架构文档 §5）：解析 Rust 源码里全部 #[tauri::command]
 * 函数名，与 contract.ts 的 IPC_COMMANDS 比对；同时确认 lib.rs 的
 * generate_handler! 注册清单覆盖全部命令。任何一侧漂移都让门禁失败。
 */

const RUST_SRC = join(import.meta.dirname, '..', '..', '..', 'src-tauri', 'src');

function listRustFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      out.push(...listRustFiles(full));
    } else if (entry.endsWith('.rs')) {
      out.push(full);
    }
  }
  return out;
}

function commandFns(source: string): string[] {
  const names: string[] = [];
  // 匹配 `#[tauri::command]` 之后（可跨越 pub、async 等修饰）的第一个 fn。
  const pattern = /#\[tauri::command\][^]*?\bfn\s+([a-z0-9_]+)/g;
  for (const match of source.matchAll(pattern)) {
    const name = match[1];
    if (name) names.push(name);
  }
  return names;
}

function handlerRegistrations(source: string): string[] {
  const block = /generate_handler!\[([^]+?)\]/.exec(source);
  if (!block || block[1] === undefined) return [];
  return [...block[1].matchAll(/([a-z0-9_]+)\s*,/g)]
    .map((m) => m[1]!)
    .filter((name) => name.includes('_'));
}

describe('IPC 合同比对', () => {
  const rustFiles = listRustFiles(RUST_SRC);
  const rustCommands = new Set<string>(
    rustFiles.flatMap((file) => commandFns(readFileSync(file, 'utf8'))),
  );
  const contractCommands = new Set<string>(IPC_COMMANDS);

  it('Rust 命令与 contract.ts 完全一致', () => {
    const missingInTs = [...rustCommands].filter((name) => !contractCommands.has(name));
    const missingInRs = [...contractCommands].filter((name) => !rustCommands.has(name));
    expect(
      {
        rustCommands: [...rustCommands].sort(),
        missingInTs,
        missingInRs,
      },
      `合同漂移：Rust 有而 TS 缺 ${JSON.stringify(missingInTs)}；TS 有而 Rust 缺 ${JSON.stringify(missingInRs)}`,
    ).toEqual({
      rustCommands: [...contractCommands].sort(),
      missingInTs: [],
      missingInRs: [],
    });
  });

  it('generate_handler 注册覆盖全部命令', () => {
    const lib = readFileSync(join(RUST_SRC, 'lib.rs'), 'utf8');
    const registered = new Set(handlerRegistrations(lib));
    const unregistered = [...contractCommands].filter((name) => !registered.has(name));
    expect(unregistered, `未注册进 generate_handler：${JSON.stringify(unregistered)}`).toEqual([]);
  });
});
