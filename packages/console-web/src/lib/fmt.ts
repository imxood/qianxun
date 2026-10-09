/** 展示格式化：纯函数，无副作用，vitest 直接覆盖（docs/20 §8 质量门禁）。 */

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86400;

/** 相对时间：刚刚 / 14 分钟前 / 3 小时 / 6 天前。超过一周回落到日期。 */
export function relativeTime(unixSeconds: number | null | undefined, now = Date.now()): string {
  if (unixSeconds == null || unixSeconds <= 0) return '—';
  const delta = Math.max(0, Math.floor(now / 1000) - unixSeconds);
  if (delta < MINUTE) return '刚刚';
  if (delta < HOUR) return `${Math.floor(delta / MINUTE)} 分钟前`;
  if (delta < DAY) return `${Math.floor(delta / HOUR)} 小时`;
  if (delta < 7 * DAY) return `${Math.floor(delta / DAY)} 天前`;
  return date(unixSeconds);
}

/** 日历日期：10-09 / 2024-10-09。 */
export function date(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  const mm = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${mm}-${dd}`;
}

/** 长日期：10-09（证书/签发这类需要年份的场合用 ISO 前 10 位）。 */
export function isoDate(unixSeconds: number | null | undefined): string {
  if (unixSeconds == null || unixSeconds <= 0) return '—';
  return new Date(unixSeconds * 1000).toISOString().slice(0, 10);
}

const UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB'] as const;

/** 字节：1.4 GiB / 218 MiB / 84 KiB。固定一位小数以上，其余不带小数。 */
export function bytes(value: number | null | undefined): string {
  if (value == null || value <= 0) return '0 B';
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < UNITS.length - 1) {
    size /= 1024;
    unit += 1;
  }
  const text = size >= 100 || Number.isInteger(size) ? String(Math.round(size)) : size.toFixed(1);
  return `${text} ${UNITS[unit]}`;
}

/** 百分比：24% / 78.5%。上限 100，避免溢出条。 */
export function percent(ratio: number): string {
  const clamped = Math.max(0, Math.min(1, ratio)) * 100;
  return Number.isInteger(clamped) ? `${clamped}%` : `${clamped.toFixed(1)}%`;
}

/** 秒 → 04:37（配对码倒计时）。 */
export function countdown(seconds: number): string {
  const safe = Math.max(0, Math.floor(seconds));
  const mm = String(Math.floor(safe / 60)).padStart(2, '0');
  const ss = String(safe % 60).padStart(2, '0');
  return `${mm}:${ss}`;
}

/** 审计时间戳：10-09 19:42:05（本地时区）。 */
export function stamp(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${date(unixSeconds)} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}
