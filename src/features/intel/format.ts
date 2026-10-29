/** 情报域展示格式化（docs/17 §10）：数据优先、mono 数字、缺省「—」。 */

/** HH:mm（今天）/ MM-DD（更早）。 */
export function formatTime(ms: number | null | undefined): string {
  if (!ms) return '';
  const date = new Date(ms);
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  const pad = (n: number) => String(n).padStart(2, '0');
  if (sameDay) return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  return `${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 紧凑数字：12.4M / 768K / 96。 */
export function formatCompact(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${Math.round(value / 1_000)}K`;
  return String(value);
}

/** 上下文窗口：1M / 128K / 32K。 */
export function formatContext(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  if (value >= 1_000_000) return `${Number((value / 1_000_000).toFixed(1))}M`;
  return `${Math.round(value / 1024)}K`;
}

/** 千 token 美元价：2.0 / 0.075 / —。 */
export function formatPrice(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  if (value >= 10) return String(Math.round(value));
  if (value >= 1) return value.toFixed(1);
  if (value >= 0.01) return value.toFixed(2);
  return value.toFixed(3);
}

/** 指数（AA 智能/编码）：一位小数或 —。 */
export function formatIndex(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  return value.toFixed(1);
}
