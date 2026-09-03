import { call } from '../lib/ipc';
import type {
  FilesPage,
  GrepOptions,
  GrepPage,
  SearchOpen,
  SearchStatus,
} from '../lib/ipc/contract';

/**
 * 搜索域状态：文件名页与内容页共享同一份索引（根目录 + 扫描状态），
 * 各自的查询与结果独立。扫描轮询只在「扫描中」时运转。
 */
class SearchStore {
  rootInput = $state('');
  status: SearchStatus | null = $state(null);
  openError = $state('');

  // 文件名搜索
  filesQuery = $state('');
  filesResult: FilesPage | null = $state(null);
  filesBusy = $state(false);

  // 内容搜索
  grepQuery = $state('');
  grepOptions = $state<GrepOptions>({
    regex: false,
    smartCase: true,
    beforeContext: 1,
    afterContext: 1,
  });
  grepResult: GrepPage | null = $state(null);
  grepBusy = $state(false);

  private pollTimer: ReturnType<typeof setInterval> | null = null;
  /** 防抖句柄（两个页面共用节奏：输入停 80ms 即查）。 */
  private filesDebounce: ReturnType<typeof setTimeout> | null = null;
  private grepDebounce: ReturnType<typeof setTimeout> | null = null;

  get scanning(): boolean {
    return this.status?.scanning ?? false;
  }

  /** 打开/切换根目录并启动状态轮询。 */
  async open(root: string): Promise<void> {
    this.openError = '';
    try {
      const opened = await call<SearchOpen>('search_open', { root });
      this.rootInput = opened.root;
      await this.refreshStatus();
      this.ensurePolling();
    } catch (error) {
      this.openError = error instanceof Error ? error.message : String(error);
    }
  }

  async refreshStatus(): Promise<void> {
    try {
      this.status = await call<SearchStatus>('search_status');
    } catch {
      // 轮询失败不打断 UI，下轮再试。
    }
  }

  private ensurePolling(): void {
    if (this.pollTimer) return;
    this.pollTimer = setInterval(() => {
      if (!this.scanning) {
        // 扫描结束：补一次终态然后停表。
        void this.refreshStatus();
        if (this.pollTimer) clearInterval(this.pollTimer);
        this.pollTimer = null;
        return;
      }
      void this.refreshStatus();
    }, 300);
  }

  /** 文件名搜索（防抖由页面触发端调用 scheduleFiles）。 */
  async runFiles(): Promise<void> {
    const query = this.filesQuery.trim();
    if (!query) {
      this.filesResult = null;
      return;
    }
    this.filesBusy = true;
    try {
      this.filesResult = await call<FilesPage>('search_files', {
        query,
        limit: 200,
        offset: 0,
      });
    } finally {
      this.filesBusy = false;
    }
  }

  scheduleFiles(): void {
    if (this.filesDebounce) clearTimeout(this.filesDebounce);
    this.filesDebounce = setTimeout(() => void this.runFiles(), 80);
  }

  /** 内容搜索：新一页替换结果；continuation 用 nextFileOffset 追加。 */
  async runGrep(continuation = false): Promise<void> {
    const query = this.grepQuery.trim();
    if (!query) {
      this.grepResult = null;
      return;
    }
    this.grepBusy = true;
    try {
      const page = await call<GrepPage>('search_content', {
        query,
        opts: this.grepOptions,
        fileOffset: continuation ? (this.grepResult?.nextFileOffset ?? 0) : 0,
      });
      this.grepResult =
        continuation && this.grepResult
          ? {
              ...page,
              items: [...this.grepResult.items, ...page.items],
            }
          : page;
    } finally {
      this.grepBusy = false;
    }
  }

  scheduleGrep(): void {
    if (this.grepDebounce) clearTimeout(this.grepDebounce);
    this.grepDebounce = setTimeout(() => void this.runGrep(), 120);
  }

  async cancelGrep(): Promise<void> {
    await call('search_cancel');
  }

  dispose(): void {
    if (this.pollTimer) clearInterval(this.pollTimer);
    if (this.filesDebounce) clearTimeout(this.filesDebounce);
    if (this.grepDebounce) clearTimeout(this.grepDebounce);
    this.pollTimer = null;
    this.filesDebounce = null;
    this.grepDebounce = null;
  }
}

export const search = new SearchStore();
