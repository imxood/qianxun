import { describe, expect, it, vi } from 'vitest';
import { multiHitMenu, singleHitMenu } from './hit-context-menu';

// vitest 不带 DOM：mock clipboard + toast
Object.assign(navigator, {
  clipboard: {
    writeText: vi.fn(async () => undefined),
  },
});

describe('singleHitMenu', () => {
  it('单行：含打开 / 定位 / 复制路径（4 种）+ 文件名', () => {
    const items = singleHitMenu('src/main.ts');
    expect(items[0]?.label).toBe('打开文件');
    expect(items[1]?.label).toBe('在资源管理器中显示');
    expect(items).toHaveLength(6); // 打开 + 定位 + 4 个复制项
    expect(items.map((i) => i.label)).toContain('复制路径');
    expect(items.map((i) => i.label)).toContain('复制路径（带引号）');
    expect(items.map((i) => i.label)).toContain('复制路径（正斜杠）');
    expect(items.map((i) => i.label)).toContain('复制文件名');
  });

  it('点击行为：调用 onclick 不抛错', () => {
    const items = singleHitMenu('a.rs');
    // mock navigator.clipboard.writeText 不存在也能跑（locate.ts 内部 try/catch）
    for (const item of items) {
      expect(() => item.onclick?.()).not.toThrow();
    }
  });
});

describe('multiHitMenu', () => {
  it('多行：只含两项（定位第一项 + 批量复制）', () => {
    const items = multiHitMenu(['a.rs', 'b.rs', 'c.rs']);
    expect(items).toHaveLength(2);
    expect(items[0]?.label).toContain('在资源管理器中显示');
    expect(items[1]?.label).toBe('复制 3 条路径');
  });

  it('空数组：仍然返回两项（locate 第一项会 noop）', () => {
    const items = multiHitMenu([]);
    expect(items).toHaveLength(2);
  });
});
