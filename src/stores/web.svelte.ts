import { harness } from './harness.svelte';

/**
 * 联网搜索域状态（R001）。
 *
 * 数据源是 DSH webServer 上桥挂载的 /qx/web/search 端点（前端直连，
 * CSP 已放行回环地址）。engine 用 settings.web 的引擎清单选择。
 */

export interface WebSearchItem {
  title: string;
  url: string;
  snippet: string;
  engine: string;
}

export interface WebSearchResponse {
  ok: boolean;
  markdown?: string;
  items?: WebSearchItem[];
  error?: string;
}

const SEARCH_TIMEOUT_MS = 30_000;

class WebStore {
  query = $state('');
  engineId = $state('');
  items: WebSearchItem[] = $state([]);
  markdown = $state('');
  busy = $state(false);
  error = $state('');
  /** 已完成过一次搜索：区分「还没搜」与「搜了但 0 结果」。 */
  done = $state(false);
  /** 请求代际：新搜索使旧响应整体作废（对齐 SearchStore.grepSeq）。 */
  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;

  get available(): boolean {
    return harness.status.phase === 'ready' && harness.proxyUrl !== null;
  }

  /** DSH 未就绪时置错误态：给出去向引导，不白屏。 */
  private requireBase(): string {
    const base = harness.proxyUrl;
    if (harness.status.phase !== 'ready' || base === null) {
      throw new Error('DSH 未运行：请到「环境」页启动后再联网搜索');
    }
    return base.replace(/\/+$/, '');
  }

  async search(): Promise<void> {
    const query = this.query.trim();
    if (!query) {
      this.items = [];
      this.markdown = '';
      this.error = '';
      this.done = false;
      return;
    }
    let base: string;
    try {
      base = this.requireBase();
    } catch (cause) {
      this.items = [];
      this.markdown = '';
      this.error = cause instanceof Error ? cause.message : String(cause);
      this.done = true;
      return;
    }
    const seq = ++this.seq;
    this.busy = true;
    this.error = '';
    try {
      const response = await fetch(base + '/qx/web/search', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          query,
          engine: this.engineId || undefined,
          limit: 10,
        }),
        signal: AbortSignal.timeout(SEARCH_TIMEOUT_MS),
      });
      const body = (await response.json()) as WebSearchResponse;
      if (seq !== this.seq) return;
      if (!response.ok || !body.ok) {
        throw new Error(body.error ?? 'HTTP ' + String(response.status));
      }
      this.items = body.items ?? [];
      this.markdown = body.markdown ?? '';
      this.done = true;
    } catch (cause) {
      if (seq !== this.seq) return;
      this.items = [];
      this.markdown = '';
      this.error = cause instanceof Error ? cause.message : String(cause);
      this.done = true;
    } finally {
      if (seq === this.seq) this.busy = false;
    }
  }

  scheduleSearch(delayMs = 300): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.search(), delayMs);
  }
}

export const web = new WebStore();
