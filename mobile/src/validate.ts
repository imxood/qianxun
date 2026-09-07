/** 配对链接校验与展示工具（纯函数，jest 可测）。 */

export function newId(): string {
  return 'c' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

const PAIR_URL_RE = /^http:\/\/[a-z0-9.-]+:\d+\/qx-gate\?token=[0-9a-f]{8,}$/i;

/** 千寻桌面端「远程访问」生成的配对链接形态。 */
export function isValidPairUrl(text: string): boolean {
  return PAIR_URL_RE.test(text.trim());
}

export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function relTime(ts: number): string {
  if (!ts) {
    return '从未使用';
  }
  const diff = Date.now() - ts;
  const minute = 60_000;
  if (diff < minute) {
    return '刚刚使用';
  }
  if (diff < 60 * minute) {
    return `${Math.floor(diff / minute)} 分钟前使用`;
  }
  if (diff < 24 * 60 * minute) {
    return `${Math.floor(diff / (60 * minute))} 小时前使用`;
  }
  return `${Math.floor(diff / (24 * 60 * minute))} 天前使用`;
}
