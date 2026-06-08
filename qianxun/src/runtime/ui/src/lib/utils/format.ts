// Stage 7a 格式化工具 — 时间/大小/字节

/**
 * 把 ISO 8601 时间字符串格式化为本地化短字符串.
 * 输入无效时返回原值.
 */
export function formatTimestamp(iso: string | undefined | null): string {
	if (!iso) return '—';
	const d = new Date(iso);
	if (isNaN(d.getTime())) return iso;
	const now = new Date();
	const diffMs = now.getTime() - d.getTime();
	const diffSec = Math.floor(diffMs / 1000);
	if (diffSec < 60) return `${diffSec}s ago`;
	const diffMin = Math.floor(diffSec / 60);
	if (diffMin < 60) return `${diffMin}m ago`;
	const diffHour = Math.floor(diffMin / 60);
	if (diffHour < 24) return `${diffHour}h ago`;
	const diffDay = Math.floor(diffHour / 24);
	if (diffDay < 30) return `${diffDay}d ago`;
	return d.toLocaleDateString();
}

/**
 * 把字节数格式化为人类可读字符串.
 */
export function formatBytes(bytes: number | undefined | null): string {
	if (bytes == null) return '—';
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/**
 * 把毫秒格式化为 "234ms" / "1.23s".
 */
export function formatLatency(ms: number | undefined | null): string {
	if (ms == null) return '—';
	if (ms < 1000) return `${Math.round(ms)}ms`;
	return `${(ms / 1000).toFixed(2)}s`;
}

/**
 * 截断字符串到指定长度, 末尾加省略号.
 */
export function truncate(text: string, max: number): string {
	if (!text) return '';
	if (text.length <= max) return text;
	return text.slice(0, max - 1) + '…';
}
