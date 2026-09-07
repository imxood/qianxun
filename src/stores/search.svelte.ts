import { Channel } from '@tauri-apps/api/core';
import { call } from '../lib/ipc';
import type {
  DriveInfo,
  FilesPage,
  GrepHit,
  GrepOptions,
  GrepPage,
  GrepProgress,
  SearchOpen,
  SearchStatus,
} from '../lib/ipc/contract';

/** 整词匹配时把查询包成 `\b(?:…)\b`：普通文本先做正则转义。 */
function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * 搜索域状态：文件名页与内容页共享同一份索引（根目录 + 扫描状态），
 * 各自的查询与结果独立。扫描轮询只在「扫描中」时运转。
 */
class SearchStore {
  rootInput = $state('');
  status: SearchStatus | null = $state(null);
  openError = $state('');
  drives: DriveInfo[] = $state([]);
  private drivesLoaded = false;

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
  /** 整词匹配（包 \b…\b，隐含正则开）。 */
  grepWholeWord = $state(false);
  /** 文件名 glob 过滤（`*.rs` / `src/**`；空 = 不过滤）。 */
  grepGlob = $state('');
  grepResult: GrepPage | null = $state(null);
  grepBusy = $state(false);
  grepError = $state('');
  /** 请求代际：过期流式分片与过期终值一律丢弃。 */
  private grepSeq = 0;

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

  /** 盘符列表（选择根目录用）：懒加载一次。 */
  async loadDrives(): Promise<void> {
    if (this.drivesLoaded) return;
    this.drivesLoaded = true;
    try {
      this.drives = await call<DriveInfo[]>('search_list_drives');
    } catch {
      // 枚举失败不阻断：用户仍可手输路径。
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

  /**
   * 内容搜索（流式）：后端分片推进、经 Channel 逐批推送，本端即到即渲染。
   * 新查询使旧代际失效（分片与终值都被丢弃）；终值以通道累计为准——
   * 它总是 ≥ 后端一次性返回的全量（分片先到齐才收终值）。
   */
  async runGrep(): Promise<void> {
    const raw = this.grepQuery.trim();
    if (!raw) {
      this.grepSeq += 1;
      this.grepResult = null;
      this.grepError = '';
      return;
    }
    const seq = ++this.grepSeq;
    // 整词：包 \b(?:…)\b 并隐含开正则（引擎无独立整词旋钮）。
    const query = this.grepWholeWord
      ? `\\b(?:${this.grepOptions.regex ? raw : escapeRegex(raw)})\\b`
      : raw;
    const opts: GrepOptions = {
      ...this.grepOptions,
      regex: this.grepOptions.regex || this.grepWholeWord,
      glob: this.grepGlob.trim() || undefined,
    };

    this.grepBusy = true;
    this.grepError = '';
    const items: GrepHit[] = [];
    let filesSearched = 0;
    let filesWithMatches = 0;
    const flush = (): void => {
      this.grepResult = {
        items,
        filesSearched,
        filesWithMatches,
        nextFileOffset: 0,
        aborted: false,
      };
    };
    flush(); // 先给空结果：列表区立刻进入「搜索中」态。

    const onProgress = new Channel<GrepProgress>();
    onProgress.onmessage = (progress) => {
      if (seq !== this.grepSeq) return; // 过期分片丢弃。
      items.push(...progress.items);
      filesSearched = Math.max(filesSearched, progress.filesSearched);
      filesWithMatches = Math.max(filesWithMatches, progress.filesWithMatches);
      flush();
    };

    try {
      const page = await call<GrepPage>('search_content', { query, opts, onProgress });
      if (seq !== this.grepSeq) return;
      // 终值兜底：通道分片若因时序遗漏，终值仍是完整集合。
      const merged = page.items.length > items.length ? page.items : items;
      this.grepResult = { ...page, items: merged };
    } catch (error) {
      if (seq === this.grepSeq) {
        this.grepError = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (seq === this.grepSeq) this.grepBusy = false;
    }
  }

  scheduleGrep(): void {
    if (this.grepDebounce) clearTimeout(this.grepDebounce);
    this.grepDebounce = setTimeout(() => void this.runGrep(), 200);
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
