import { describe, expect, it } from 'vitest';
import { summarizeSyncResults } from './sync';
import type { SyncItemResult } from '../ipc/contract';

/** 清单同步汇总文案（08 设计 §8 vitest：同步结果渲染的文案逻辑）。 */
function item(partial: Partial<SyncItemResult>): SyncItemResult {
  return { name: 'pkg', ok: true, skipped: false, detail: '已安装 1.0.0', ...partial };
}

describe('summarizeSyncResults', () => {
  it('全部成功 → 全部就绪', () => {
    const results = [item({ name: 'a' }), item({ name: 'b', skipped: true, detail: '已是 1.0.0' })];
    expect(summarizeSyncResults(results)).toBe('全部就绪。');
  });

  it('部分失败 → 说清几个就绪 + 重试路径（G4）', () => {
    const results = [
      item({ name: 'a' }),
      item({ name: 'b', skipped: true, detail: '已是 1.0.0' }),
      item({ name: 'c', ok: false, detail: 'registry 上没有 c@1.0.0' }),
    ];
    expect(summarizeSyncResults(results)).toBe(
      '其余 2 项已就绪。失败项可再次点击「同步清单」重试，已成功项会跳过。',
    );
  });

  it('全部失败 → 0 项就绪，同样给出重试指引', () => {
    const results = [item({ name: 'a', ok: false, detail: 'x' })];
    expect(summarizeSyncResults(results)).toBe(
      '其余 0 项已就绪。失败项可再次点击「同步清单」重试，已成功项会跳过。',
    );
  });

  it('空结果 → 全部就绪（空清单同步是空操作）', () => {
    expect(summarizeSyncResults([])).toBe('全部就绪。');
  });
});
