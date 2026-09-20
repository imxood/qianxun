/**
 * 清单同步结果的汇总文案（08 设计 §4.1 / §6）。
 *
 * 独立成纯函数以便单测：底部一行汇总告诉用户「成了几个、败了几个、
 * 失败了怎么办」。单个失败不影响其余（G4），文案必须把重试路径说清——
 * 再次点击同步清单，已成功项会按幂等跳过。
 */
import type { SyncItemResult } from '../ipc/contract';

export function summarizeSyncResults(results: SyncItemResult[]): string {
  const failed = results.filter((item) => !item.ok);
  if (failed.length === 0) return '全部就绪。';
  return `其余 ${results.length - failed.length} 项已就绪。失败项可再次点击「同步清单」重试，已成功项会跳过。`;
}
