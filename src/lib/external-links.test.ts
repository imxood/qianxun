import { describe, expect, it } from 'vitest';
import { isShellReady, parseShellLink } from './external-links';

const GATEWAY = 'http://127.0.0.1:23090';

/** 构造一条与注入脚本同形的 open-url 消息。 */
function openUrl(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    __qxShell: '__qxShell',
    kind: 'open-url',
    url: 'https://example.com/page',
    ctrl: false,
    middle: false,
    shift: false,
    ...overrides,
  };
}

describe('isShellReady', () => {
  it('识别注入脚本的 ready 宣告', () => {
    expect(isShellReady({ __qxShell: '__qxShell', kind: 'ready' })).toBe(true);
  });

  it('其他 kind / 缺标记 / 非对象一律拒绝', () => {
    expect(isShellReady({ __qxShell: '__qxShell', kind: 'open-url' })).toBe(false);
    expect(isShellReady({ kind: 'ready' })).toBe(false);
    expect(isShellReady('ready')).toBe(false);
    expect(isShellReady(null)).toBe(false);
  });
});

describe('parseShellLink', () => {
  it('解析合法消息并绝对化 url', () => {
    const link = parseShellLink(GATEWAY, openUrl({ url: 'https://example.com/a?b=1' }), GATEWAY);
    expect(link).toEqual({
      url: 'https://example.com/a?b=1',
      ctrl: false,
      middle: false,
      shift: false,
    });
  });

  it('相对地址按 iframe 页面语境无法解析——非绝对 url 拒绝', () => {
    expect(parseShellLink(GATEWAY, openUrl({ url: '/path' }), GATEWAY)).toBeNull();
  });

  it('origin 不匹配网关来源时拒绝（防任意父页/伪造来源）', () => {
    expect(parseShellLink('https://evil.com', openUrl(), GATEWAY)).toBeNull();
  });

  it('网关未知（DSH 未就绪）时一律拒绝', () => {
    expect(parseShellLink(GATEWAY, openUrl(), null)).toBeNull();
  });

  it('非 http/https 协议拒绝', () => {
    expect(parseShellLink(GATEWAY, openUrl({ url: 'file:///C:/x' }), GATEWAY)).toBeNull();
    expect(parseShellLink(GATEWAY, openUrl({ url: 'javascript:alert(1)' }), GATEWAY)).toBeNull();
  });

  it('修饰键仅认布尔 true', () => {
    const link = parseShellLink(GATEWAY, openUrl({ ctrl: true, middle: 1, shift: 'yes' }), GATEWAY);
    expect(link).toEqual({
      url: 'https://example.com/page',
      ctrl: true,
      middle: false,
      shift: false,
    });
  });

  it('url 缺失或非字符串拒绝', () => {
    expect(parseShellLink(GATEWAY, openUrl({ url: undefined }), GATEWAY)).toBeNull();
    expect(parseShellLink(GATEWAY, openUrl({ url: 42 }), GATEWAY)).toBeNull();
  });
});
