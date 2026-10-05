/**
 * 笔记自动保存调度测试（docs/18 B3）。
 *
 * 这是笔记域的**首批**前端用例（此前 notes 前端 0 覆盖，数据丢失类缺陷
 * 只能靠人工发现）。测的正是 S8 修复的不变量：
 * 「保存正在飞行时，防抖到期必须重排而不是静默丢弃这一轮编辑」。
 */
import { describe, expect, it } from 'vitest';

import {
  AUTOSAVE_DEBOUNCE_MS,
  autosaveDecision,
  needsUnloadPrompt,
  type AutosaveState,
} from './autosave';

const clean: AutosaveState = { dirty: false, saving: false, hasActive: true };
const dirtyIdle: AutosaveState = { dirty: true, saving: false, hasActive: true };
const dirtySaving: AutosaveState = { dirty: true, saving: true, hasActive: true };

describe('autosaveDecision（docs/18 S8 核心不变量）', () => {
  it('有改动 + 无保存在飞 → 触发保存', () => {
    expect(autosaveDecision(dirtyIdle)).toEqual({ kind: 'save' });
  });

  it('有改动 + 保存在飞 → 重排，而不是丢弃（S8 修复点）', () => {
    // 旧实现在此分支直接 return 且不重排 → 这轮编辑永不落盘。
    expect(autosaveDecision(dirtySaving)).toEqual({ kind: 'retry' });
  });

  it('无改动 → 空闲', () => {
    expect(autosaveDecision(clean)).toEqual({ kind: 'idle', reason: 'clean' });
  });

  it('无选中笔记时即便 dirty 也不保存', () => {
    expect(autosaveDecision({ dirty: true, saving: false, hasActive: false })).toEqual({
      kind: 'idle',
      reason: 'no-active',
    });
  });

  it('保存中但已无改动 → 空闲（不空转重排）', () => {
    // dirty 判定优先于 saving：无改动时不该再排一轮空 timer。
    expect(autosaveDecision({ dirty: false, saving: true, hasActive: true })).toEqual({
      kind: 'idle',
      reason: 'clean',
    });
  });

  it('无选中优先于 dirty（不会向不存在的笔记发保存）', () => {
    expect(autosaveDecision({ dirty: true, saving: true, hasActive: false }).kind).toBe('idle');
  });

  it('连续保存飞行不会升级为 save（必须等落地）', () => {
    // 状态机语义：retry 不是 save 的别名，重排后仍由新的 tick 重新判断。
    let state = dirtySaving;
    for (let i = 0; i < 5; i += 1) {
      expect(autosaveDecision(state).kind).toBe('retry');
    }
    // 保存落地后重排的那一轮才会真正保存。
    state = dirtyIdle;
    expect(autosaveDecision(state).kind).toBe('save');
  });
});

describe('needsUnloadPrompt（卸载/关窗提示）', () => {
  it('有未落盘编辑且无保存在飞 → 提示', () => {
    expect(needsUnloadPrompt(dirtyIdle)).toBe(true);
  });

  it('保存在飞 → 不提示（数据会自己落盘）', () => {
    expect(needsUnloadPrompt(dirtySaving)).toBe(false);
  });

  it('无改动 → 不提示', () => {
    expect(needsUnloadPrompt(clean)).toBe(false);
  });

  it('无选中 → 不提示', () => {
    expect(needsUnloadPrompt({ dirty: true, saving: false, hasActive: false })).toBe(false);
  });
});

describe('防抖窗口', () => {
  it('默认 1.5s（与 docs/18 记录的旧值一致，不改变用户可感知的节奏）', () => {
    expect(AUTOSAVE_DEBOUNCE_MS).toBe(1500);
  });
});
