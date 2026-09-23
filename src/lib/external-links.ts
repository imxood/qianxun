import { openUrl } from '@tauri-apps/plugin-opener';
import { call } from './ipc';
import type { WindowOpenExternalResult } from './ipc/contract';

/**
 * DSH 页（回环网关 iframe）↔ 千寻外壳的外链桥。
 *
 * 背景：WebView2（wry 未注册 NewWindowRequested 回调时）静默吞掉一切弹窗
 * 请求，DSH 聊天里 target="_blank" 的链接点了毫无反应；跨站 iframe 又够
 * 不到 Tauri IPC。解法：回环网关给 DSH 页注入拦截脚本
 * （src-tauri/src/remote/assets/qx-shell-links.js，协议 TAG 与此对齐），
 * 外链点击经 postMessage 上来，这里校验来源后分流：
 * - 普通点击 → 内置浏览器窗单例换址（window_open_external，千寻即浏览器）；
 * - Ctrl/⌘+点击 → 系统浏览器（opener 插件，capabilities 仅放行 http/https）；
 * - 中键/Shift+点击 → 内置浏览器窗新开后台风（不抢焦点，级联错位）。
 */

/** postMessage 协议标记（与注入脚本的 TAG 一致）。 */
const TAG = '__qxShell';

/** 注入脚本上来的外链点击（字段已在解析时校验过类型）。 */
export interface ShellLinkDispatch {
  /** 绝对化的 http/https 链接。 */
  url: string;
  /** Ctrl/⌘ 修饰：系统浏览器兜底。 */
  ctrl: boolean;
  /** 中键：内置窗新开。 */
  middle: boolean;
  /** Shift：内置窗新开。 */
  shift: boolean;
}

function isTagged(data: unknown, kind: string): data is Record<string, unknown> {
  return (
    typeof data === 'object' &&
    data !== null &&
    (data as Record<string, unknown>)[TAG] === TAG &&
    (data as Record<string, unknown>).kind === kind
  );
}

/**
 * 是否为注入脚本的 ready 宣告（外壳收到后回 arm，握手才算建立；
 * 双向发现消除「谁先加载」的时序问题）。
 */
export function isShellReady(data: unknown): boolean {
  return isTagged(data, 'ready');
}

/**
 * 校验并解析一条来自 DSH iframe 的 open-url 消息。
 * `origin` 必须等于回环网关 origin（iframe 的 src 来源，由调用方从
 * harness.proxyUrl 推导传入）；协议字段必须齐全、url 必须是合法的
 * http/https 绝对地址。任何不满足都返回 null（调用侧静默忽略）。
 */
export function parseShellLink(
  origin: string,
  data: unknown,
  expectedOrigin: string | null,
): ShellLinkDispatch | null {
  if (!expectedOrigin || origin !== expectedOrigin) return null;
  if (!isTagged(data, 'open-url')) return null;
  const rawUrl = data.url;
  if (typeof rawUrl !== 'string') return null;
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
  return {
    url: parsed.href,
    ctrl: data.ctrl === true,
    middle: data.middle === true,
    shift: data.shift === true,
  };
}

/**
 * 分流执行一条外链：见模块注释的分流表。失败向上抛（调用方决定提示方式），
 * 与 WebPage/RemotePage 对 openUrl 的容错风格一致。
 */
export async function openShellLink(link: ShellLinkDispatch): Promise<void> {
  if (link.ctrl) {
    await openUrl(link.url);
    return;
  }
  await call<WindowOpenExternalResult>('window_open_external', {
    url: link.url,
    newWindow: link.middle || link.shift,
  });
}
