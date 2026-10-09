/** `/console/api/*` 的类型与取数封装。
 *
 * 契约原则（docs/20 §4）：一页一次请求，不一行一个端点——3 秒轮询下 round-trip
 * 数会爆炸。所有失败都落到 ApiError.message，页面直接显示，不再各写一套解析。
 */

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly retryAfter?: number,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    ...init,
    headers: init?.body ? { 'content-type': 'application/json' } : undefined,
  });
  if (!response.ok) {
    const retryAfter = Number(response.headers.get('retry-after'));
    let message = `请求失败（${response.status}）`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // 非 JSON 错误体（如反代的 502 页面）：保留状态码口径的兜底文案。
    }
    throw new ApiError(
      message,
      response.status,
      Number.isFinite(retryAfter) ? retryAfter : undefined,
    );
  }
  return (await response.json()) as T;
}

const post = <T>(path: string, body?: unknown) =>
  request<T>(path, { method: 'POST', body: body === undefined ? undefined : JSON.stringify(body) });

export const api = {
  login: (username: string, password: string) =>
    post<{ ok: true; username: string }>('/console/api/login', { username, password }),
  logout: () => post<{ ok: true }>('/console/api/logout'),
  me: () => request<{ ok: true; username: string }>('/console/api/me'),

  devices: () => request<{ devices: Device[] }>('/console/api/devices'),
  issuePair: () =>
    post<{ code: string; expiresAt: number; pairUrl: string }>('/console/api/devices/pair'),
  pairStatus: (code: string) =>
    request<PairState>(`/console/api/devices/pair/status?code=${encodeURIComponent(code)}`),
  revoke: (id: string) => post<{ ok: true }>(`/console/api/devices/${id}/revoke`),
  restore: (id: string) => post<{ ok: true }>(`/console/api/devices/${id}/restore`),
  revokeAll: () => post<{ revoked: number }>('/console/api/devices/revoke-all'),
  rotate: (id: string) => post<RotatedCredential>(`/console/api/devices/${id}/rotate`),

  overview: () => request<Overview>('/console/api/overview'),
  audit: (query: AuditQuery) => request<AuditPage>(`/console/api/audit${toQuery(query)}`),
  auditCsvUrl: (query: AuditQuery) => `/console/api/audit.csv${toQuery(query)}`,
};

function toQuery(query: AuditQuery): string {
  const params = new URLSearchParams();
  if (query.type) params.set('type', query.type);
  if (query.from) params.set('from', String(query.from));
  if (query.cursor) params.set('cursor', query.cursor);
  const text = params.toString();
  return text ? `?${text}` : '';
}

// ---- 服务端返回形状（camelCase，与 docs/19 控制面一致） ----

export interface DeviceSession {
  instanceId: string;
  instanceName: string;
  class: 'interactive' | 'page' | 'bulk';
  seconds: number;
  bytesPerSec: number;
}

export interface DeviceCredential {
  hint: string;
  issuedAt: number | null;
  expiresAt: number | null;
}

/** 轮换后的凭据：`token` 明文只在这一条响应里出现，之后再也取不回。 */
export interface RotatedCredential {
  token: string;
  hint: string;
  issuedAt: number;
  expiresAt: number;
}

export interface Device {
  id: string;
  name: string;
  createdAt: number;
  lastSeenAt: number;
  revoked: boolean;
  lastIp: string | null;
  /** 24h 流量（字节）；全页唯一的设备级流量数字。 */
  bytes24h: number;
  /** 占配额比 0..1（该设备流量 / 全站 24h 流量）。 */
  share: number;
  /** 24h 交互/页面两类的占比形状，供 sparkline 用。 */
  shape: number[];
  sessions: DeviceSession[];
  credential: DeviceCredential;
}

export type PairStatus = 'pending' | 'used' | 'expired';

export interface PairState {
  status: PairStatus;
  /** 已兑换时给出设备名（状态机终态，docs/20 §4.4）。 */
  deviceName: string | null;
  expiresAt: number;
}

export interface Kpi {
  instancesOnline: number;
  instancesTotal: number;
  instancesOfflineFor: number | null;
  activeDevices: number;
  bytes24h: number;
  throttled24h: number;
  interactiveThrottled24h: number;
}

export interface Waterline {
  interactive: { level: number; cap: number };
  page: { level: number; cap: number };
  bulk: { level: number; cap: number };
  bandwidth: { level: number; cap: number };
}

export interface OverviewInstance {
  id: string;
  name: string;
  online: boolean;
  version: string | null;
  connectedFor: number | null;
  devices: string[];
  bytes1h: number;
  status: 'ok' | 'limited' | 'offline';
}

export interface Overview {
  kpi: Kpi;
  waterline: Waterline;
  instances: OverviewInstance[];
  /** 24 个小时桶，每桶一类交互字节 + 页面/批量字节。 */
  traffic: { interactive: number[]; other: number[] };
  generatedAt: number;
}

export interface AuditEvent {
  id: number;
  at: number;
  event: string;
  instanceId: string | null;
  deviceId: string | null;
  detail: Record<string, unknown>;
}

export interface AuditPage {
  events: AuditEvent[];
  nextCursor: string | null;
}

export interface AuditQuery {
  type?: string;
  from?: number;
  cursor?: string;
}
