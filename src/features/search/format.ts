/** 搜索结果列的紧凑格式化（RootBar 盘符提示与 FilesPage 结果表共用）。 */

/** 字节数 → 紧凑容量。 */
export function formatBytes(bytes: number): string {
  if (!bytes) return '—';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)}${units[unit]}`;
}

/** 毫秒时间戳 → `YYYY-MM-DD HH:mm`（0 = 未知）。 */
export function formatTime(ms: number): string {
  if (!ms) return '—';
  const date = new Date(ms);
  const pad = (value: number): string => String(value).padStart(2, '0');
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ` +
    `${pad(date.getHours())}:${pad(date.getMinutes())}`
  );
}
