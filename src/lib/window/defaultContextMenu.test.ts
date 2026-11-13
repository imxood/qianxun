import { describe, expect, it, vi } from 'vitest';
import { buildDefaultMenuItems, type DefaultMenuContext } from './defaultContextMenu';

// vitest 跑在 node 环境（无 DOM）：只测纯函数分支，不触发真实剪贴板 / execCommand。
const writeText = vi.fn(async () => undefined);
Object.assign(navigator, {
  clipboard: {
    writeText,
    readText: vi.fn(async () => ''),
  },
});

/** 构造一个上下文（element 留空：纯函数不依赖它）。 */
function ctx(partial: Partial<DefaultMenuContext>): DefaultMenuContext {
  return {
    editable: false,
    selection: '',
    hasSelection: false,
    element: null,
    ...partial,
  };
}

/** 取菜单项 label 列表。 */
function labels(items: ReturnType<typeof buildDefaultMenuItems>): string[] {
  return items.map((item) => item.label);
}

describe('buildDefaultMenuItems', () => {
  it('可编辑 + 有选区：剪切/复制/粘贴/全选，全部可用', () => {
    const items = buildDefaultMenuItems(
      ctx({ editable: true, selection: 'hello', hasSelection: true }),
    );
    expect(labels(items)).toEqual(['剪切', '复制', '粘贴', '全选']);
    expect(items.every((item) => item.disabled !== true)).toBe(true);
  });

  it('可编辑 + 无选区：剪切与复制禁用，粘贴与全选可用', () => {
    const items = buildDefaultMenuItems(
      ctx({ editable: true, selection: '', hasSelection: false }),
    );
    expect(labels(items)).toEqual(['剪切', '复制', '粘贴', '全选']);
    expect(items[0]?.disabled).toBe(true);
    expect(items[1]?.disabled).toBe(true);
    expect(items[2]?.disabled).toBe(false);
    expect(items[3]?.disabled).toBeUndefined();
  });

  it('不可编辑 + 有选区：复制与全选', () => {
    const items = buildDefaultMenuItems(ctx({ selection: '选中文字', hasSelection: true }));
    expect(labels(items)).toEqual(['复制', '全选']);
    expect(items[0]?.disabled).toBe(false);
  });

  it('不可编辑 + 无选区：空数组（调用方仍然 preventDefault，只是不弹菜单）', () => {
    const items = buildDefaultMenuItems(ctx({}));
    expect(items).toEqual([]);
  });

  it('点击复制：把选区写入剪贴板', () => {
    writeText.mockClear();
    const items = buildDefaultMenuItems(ctx({ selection: '要复制的内容', hasSelection: true }));
    items[0]?.onclick?.();
    expect(writeText).toHaveBeenCalledWith('要复制的内容');
  });
});
