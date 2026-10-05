/**
 * NavStore 接线层冒烟测试（docs/18 B1）。
 *
 * 背景：本次全量分析发现「测试全绿 ≠ 功能能跑」——`nav.detached` 长期是
 * 布尔且**零测试覆盖**，重复分离后关闭任一窗口会让主窗重新挂载一个活的
 * DSH iframe（同一 view 两个活实例）。缺陷正好落在「对象之间怎么接线」，
 * 而当时的测试只覆盖纯函数。
 *
 * 这里补的正是那一层：store 与调用方（App.svelte / SideNav.svelte）之间
 * 的状态契约。
 */
import { describe, expect, it } from 'vitest';

import { nav, type PageId } from './nav.svelte';

/** 直接重置（class instance，字段可写）。 */
function resetNav(): void {
  nav.page = 'env';
  nav.visited = { env: true };
  nav.detached = {};
}

describe('nav.detached 引用计数（docs/18 S6）', () => {
  it('首次分离置位，并把当前页让回 env', () => {
    resetNav();
    nav.page = 'dsh';
    nav.detach('dsh');
    expect(nav.detached.dsh).toBe(1);
    expect(nav.page, '分离时若正停在被分离页，应让位').toBe('env');
  });

  it('重复分离累加计数（不静默吞掉第二次）', () => {
    resetNav();
    nav.detach('dsh');
    nav.detach('dsh');
    expect(nav.detached.dsh).toBe(2);
  });

  it('关闭一个窗口不恢复侧栏项——另一扇还开着', () => {
    resetNav();
    nav.detach('dsh');
    nav.detach('dsh');
    nav.reattach('dsh'); // 关掉其中一扇
    expect(
      nav.detached.dsh,
      '还有一扇独立窗在，主窗不得重新挂载 DSH iframe（否则同 view 双活实例）',
    ).toBe(1);
  });

  it('关闭最后一扇才恢复侧栏项', () => {
    resetNav();
    nav.detach('dsh');
    nav.reattach('dsh');
    expect(nav.detached.dsh).toBeUndefined();
  });

  it('reattach 未登记的 page 不产生负数', () => {
    resetNav();
    nav.reattach('dsh');
    expect(nav.detached.dsh).toBeUndefined();
  });

  it('计数 > 1 时连续关闭逐级递减', () => {
    resetNav();
    nav.detach('dsh');
    nav.detach('dsh');
    nav.detach('dsh');
    nav.reattach('dsh');
    expect(nav.detached.dsh).toBe(2);
    nav.reattach('dsh');
    expect(nav.detached.dsh).toBe(1);
    nav.reattach('dsh');
    expect(nav.detached.dsh).toBeUndefined();
  });
});

describe('nav 三态与调用方的真值契约（docs/18 B1）', () => {
  // App.svelte:113 / SideNav.svelte:93,121 都是真值判断，数字类型必须
  // 仍能正确表达「已分离 / 未分离」，否则这次类型改动会静默破��导航。
  it('真值语义：计数 0/undefined 为假，正数为真', () => {
    resetNav();
    expect(!nav.detached.dsh).toBe(true); // 侧栏项应可见
    nav.detach('dsh');
    expect(!!nav.detached.dsh).toBe(true); // 侧栏项应让位
    nav.reattach('dsh');
    expect(!nav.detached.dsh).toBe(true);
  });

  it('分离不影响其它页', () => {
    resetNav();
    nav.visit('notes');
    nav.detach('dsh');
    expect(nav.detached.notes).toBeUndefined();
    expect(nav.visited.notes, 'keep-alive 标记不受分离影响').toBe(true);
  });

  it('visit 幂等且 detach 后再 visit 不复活容器', () => {
    resetNav();
    nav.visit('dsh');
    nav.detach('dsh');
    nav.visit('dsh'); // DSH ready 后的后台预热会再 visit
    expect(nav.visited.dsh).toBe(true);
    // App.svelte 的条件是 visited && !detached —— detached 仍在则不挂载
    expect(!!(nav.visited.dsh && !nav.detached.dsh)).toBe(false);
  });

  it('go 只改当前页，不动 visited/detached', () => {
    resetNav();
    nav.visit('intel');
    nav.detach('dsh');
    nav.go('files');
    expect(nav.page).toBe('files');
    expect(nav.visited.intel).toBe(true);
    expect(nav.detached.dsh).toBe(1);
  });

  it('所有 PageId 都是合法键（防止重构时漏分支）', () => {
    resetNav();
    const ids: PageId[] = [
      'env',
      'dsh',
      'files',
      'web',
      'intel',
      'remote',
      'notes',
      'games',
      'plugins',
      'settings',
    ];
    for (const id of ids) {
      nav.detach(id);
      expect(nav.detached[id], `${id} 应可分离`).toBe(1);
      nav.reattach(id);
      expect(nav.detached[id], `${id} 应可恢复`).toBeUndefined();
    }
  });
});
