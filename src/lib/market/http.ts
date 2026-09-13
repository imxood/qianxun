/** 市场浏览的共享 fetch：超时 + 非 2xx 报错（消息与 Rust 侧风格一致）。 */

const TIMEOUT_MS = 20_000;

export async function fetchJson(url: string): Promise<unknown> {
  let response: Response;
  try {
    response = await fetch(url, {
      signal: AbortSignal.timeout(TIMEOUT_MS),
      headers: { accept: 'application/json' },
    });
  } catch (cause) {
    throw new Error(`registry 不可达：${cause instanceof Error ? cause.message : String(cause)}`, {
      cause,
    });
  }
  if (!response.ok) throw new Error(`registry 返回 HTTP ${String(response.status)}`);
  return response.json();
}

/** 包一层「下载二进制」：同样超时与非 2xx 报错。 */
export async function fetchBytes(url: string): Promise<ArrayBuffer> {
  let response: Response;
  try {
    response = await fetch(url, { signal: AbortSignal.timeout(TIMEOUT_MS) });
  } catch (cause) {
    throw new Error(`下载失败：${cause instanceof Error ? cause.message : String(cause)}`, {
      cause,
    });
  }
  if (!response.ok) throw new Error(`下载失败：HTTP ${String(response.status)}`);
  return response.arrayBuffer();
}
