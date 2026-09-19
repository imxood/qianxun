import { invoke } from '@tauri-apps/api/core';
import type { IpcCommand } from './contract';

/**
 * 统一的 IPC 调用封装（编码规范 §7）：前端只准从这里过。
 * 把 Tauri 的字符串错误包成带命令上下文的 AppError，UI 层据 `command`
 * 与 `message` 决定展示方式，禁止裸吞。
 */
export class AppError extends Error {
  readonly command: IpcCommand;

  constructor(command: IpcCommand, message: string) {
    super(message);
    this.name = 'AppError';
    this.command = command;
  }
}

function toMessage(raw: unknown): string {
  if (typeof raw === 'string') return raw;
  if (raw instanceof Error) return raw.message;
  // Rust 侧 serde 错误对象等：尽量抽出可读字段，抽不出就整体字符串化。
  if (raw && typeof raw === 'object') {
    const record = raw as Record<string, unknown>;
    for (const key of ['message', 'error', 'kind']) {
      const value = record[key];
      if (typeof value === 'string') return value;
    }
    try {
      return JSON.stringify(raw);
    } catch {
      return '未知错误（错误对象无法序列化）';
    }
  }
  return String(raw);
}

export async function call<T>(command: IpcCommand, args?: Record<string, unknown>): Promise<T> {
  const started = performance.now();
  // dev 模式下输出结构化日志（汇总 §7.2 + 04 §3.1）：前端 IPC 域零埋点
  // → Chrome DevTools console 即是排查通道；release build 自动 tree-shake 掉。
  if (import.meta.env.DEV) {
    console.debug(`[ipc] → ${command}`, args);
  }
  try {
    const result = await invoke<T>(command, args);
    if (import.meta.env.DEV) {
      const elapsed = (performance.now() - started).toFixed(1);
      console.debug(`[ipc] ← ${command} ok latency_ms=${elapsed}`);
    }
    return result;
  } catch (error) {
    const message = toMessage(error);
    if (import.meta.env.DEV) {
      const elapsed = (performance.now() - started).toFixed(1);
      console.error(`[ipc] × ${command} failed latency_ms=${elapsed}`, message);
    }
    throw new AppError(command, message);
  }
}
