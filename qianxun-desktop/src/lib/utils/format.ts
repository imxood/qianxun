// qianxun-desktop/src/lib/utils/format.ts

export function formatRelativeTime(iso: string): string {
	const now = Date.now();
	const then = new Date(iso).getTime();
	const diff = Math.floor((now - then) / 1000);
	if (diff < 60) return `${diff}s ago`;
	if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
	if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
	if (diff < 604800) return `${Math.floor(diff / 86400)} d ago`;
	return new Date(iso).toLocaleDateString();
}

export function formatDuration(seconds: number): string {
	if (seconds < 60) return `${seconds}s`;
	if (seconds < 3600) return `${Math.floor(seconds / 60)} min`;
	return `${Math.floor(seconds / 3600)} h ${Math.floor((seconds % 3600) / 60)} min`;
}

export function truncate(s: string, max: number): string {
	return s.length > max ? s.slice(0, max) + '…' : s;
}
