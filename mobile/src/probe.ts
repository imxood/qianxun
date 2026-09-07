import type { Connection, PcInfo } from './types';

const PROBE_TIMEOUT_MS = 3000;

export function infoUrl(conn: Connection): string | null {
  try {
    const parsed = new URL(conn.url);
    const token = parsed.searchParams.get('token') ?? '';
    return `${parsed.origin}/qx-mobile/info?token=${encodeURIComponent(token)}`;
  } catch {
    return null;
  }
}

/**
 * 在线探测。RN 的原生 fetch 没有 CORS 限制；配对前 cookie 不在 fetch 的
 * cookie jar（在 WebView 里），因此鉴权走 query token（网关 info 支持）。
 */
export async function probeInfo(conn: Connection): Promise<PcInfo | null> {
  const url = infoUrl(conn);
  if (!url) {
    return null;
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);
  try {
    const response = await fetch(url, { signal: controller.signal });
    if (!response.ok) {
      return null;
    }
    return (await response.json()) as PcInfo;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}
