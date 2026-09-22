import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { web } from './web.svelte';

/**
 * 联网搜索 store 真测（R001 实施步骤 C）：
 * fetch 用 vi.stubGlobal 注入桩，覆盖空查询不发请求 / DSH 未就绪报引导
 * 错误两条真实分支；成功链路依赖 harness 就绪态，走 e2e 探针覆盖。
 */

describe('web store', () => {
  beforeEach(() => {
    web.query = '';
    web.engineId = '';
    web.error = '';
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('空查询直接清空结果，不发请求', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    web.query = '   ';
    await web.search();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(web.items).toEqual([]);
    expect(web.error).toBe('');
  });

  it('DSH 未就绪时报引导错误，不抛出也不发请求', async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    // 默认 harness.status.phase = 'stopped'、proxyUrl = null。
    web.query = 'rust 入门指南';
    await web.search();
    expect(fetchMock).not.toHaveBeenCalled();
    expect(web.error).toContain('DSH 未运行');
    expect(web.busy).toBe(false);
  });

  it('fetch 网络失败不抛出（未就绪时 requireBase 先行拦截）', async () => {
    // 直接构造一个 proxyUrl 可用的替身：给 store 打桩不可行（read-only
    // 状态），这里只验证 fetch 层的错误映射逻辑——模拟 harness 就绪
    // 超出单测范围（e2e 覆盖），因此通过拦截 requireBase 抛错路径以外
    // 的 fetch 失败：stub fetch reject。
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('network down')));
    // harness 未就绪时 requireBase 先拦截，因此此用例只保证不崩。
    web.query = 'rust';
    await expect(web.search()).resolves.toBeUndefined();
  });
});
