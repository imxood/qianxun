/** 插件市场行内/对话框共用的格式化与徽标。 */

import type { MarketDetail } from './types';

export function count(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

export function filesize(bytes: number | null): string {
  if (bytes === null) return '';
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

/** 兼容徽标：文案 + 配色（未声明也明说，不装糊涂）。 */
export function compatBadge(detail: MarketDetail): { text: string; cls: string } {
  switch (detail.compatibility.state) {
    case 'compatible':
      return {
        text: `兼容 DSH ${detail.compatibility.requirement}`,
        cls: 'bg-ok/15 text-ok',
      };
    case 'incompatible':
      return {
        text: `不兼容 · ${detail.compatibility.reason}`,
        cls: 'bg-danger/10 text-danger',
      };
    default:
      return { text: '未声明 DSH 版本', cls: 'bg-accent-soft text-muted' };
  }
}
