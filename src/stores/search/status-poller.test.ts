import { describe, expect, it, vi } from 'vitest';
import { StatusPoller } from './status-poller';

describe('StatusPoller', () => {
  it('start 是幂等的：重复 start 不会创建第二个 handle', () => {
    let intervals = 0;
    const poller = new StatusPoller({
      intervalMs: 100,
      isActive: () => false,
      onTick: () => undefined,
      setIntervalFn: () => {
        intervals += 1;
        return Symbol(`handle-${intervals}`) as unknown as ReturnType<typeof setInterval>;
      },
    });
    poller.start();
    poller.start();
    poller.start();
    expect(intervals).toBe(1);
    poller.stop();
  });

  it('stop 后 handle 清零；onStop 调一次', () => {
    let cleared = 0;
    let handle: ReturnType<typeof setInterval> | null = null;
    let stopped = 0;
    const poller = new StatusPoller({
      intervalMs: 100,
      isActive: () => true,
      onTick: () => undefined,
      onStop: () => {
        stopped += 1;
      },
      setIntervalFn: () => {
        handle = Symbol('h') as unknown as ReturnType<typeof setInterval>;
        return handle!;
      },
      clearIntervalFn: (h) => {
        cleared += 1;
        expect(h).toBe(handle);
      },
    });
    poller.start();
    expect(poller.running).toBe(true);
    poller.stop();
    expect(poller.running).toBe(false);
    expect(cleared).toBe(1);
    expect(stopped).toBe(1);
    // 再 stop 一次幂等。
    poller.stop();
    expect(cleared).toBe(1);
    expect(stopped).toBe(1);
  });

  it('tick 内 isActive=true 时继续轮询，不停表', async () => {
    const onTick = vi.fn();
    const poller = new StatusPoller({
      intervalMs: 100,
      isActive: () => true,
      onTick,
    });
    await poller.tick();
    expect(onTick).toHaveBeenCalledTimes(1);
    expect(poller.running).toBe(false); // 没 start 就没有 handle
  });

  it('tick 内 isActive=false 时 onTick 再调一次（补终态）后停表', async () => {
    const onTick = vi.fn();
    const onStop = vi.fn();
    let handle: ReturnType<typeof setInterval> | null = null;
    const poller = new StatusPoller({
      intervalMs: 100,
      isActive: () => false,
      onTick,
      onStop,
      setIntervalFn: () => {
        // 测试用 handle：符号也能当 handle，重要的是 clearIntervalFn 被调用。
        handle = Symbol('h') as unknown as ReturnType<typeof setInterval>;
        return handle!;
      },
      clearIntervalFn: () => undefined,
    });
    poller.start();
    await poller.tick();
    expect(onTick).toHaveBeenCalledTimes(2); // 第一次 + 补终态
    expect(onStop).toHaveBeenCalledTimes(1);
    expect(poller.running).toBe(false);
  });

  it('onTick 抛错时不阻断（汇总 §03 P13）', async () => {
    const onTick = vi.fn().mockRejectedValue(new Error('boom'));
    const poller = new StatusPoller({
      intervalMs: 100,
      isActive: () => true,
      onTick,
    });
    await expect(poller.tick()).resolves.toBeUndefined();
    expect(onTick).toHaveBeenCalledTimes(1);
  });
});
