import { Channel } from '@tauri-apps/api/core';
import { call } from '../lib/ipc';
import type {
  DriveInfo,
  FileHit,
  FilesPage,
  GrepOptions,
  GrepPage,
  GrepProgress,
  SearchFilesMode,
  SearchOpen,
  SearchStatus,
} from '../lib/ipc/contract';
import { mergeGrepStream } from './search/grep-merge';
import { StatusPoller } from './search/status-poller';

/** 整词匹配时把查询包成 `\b(?:…)\b`：普通文本先做正则转义。 */
function escapeRegex(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * 搜索域状态：文件名页与内容页共享同一份索引（根目录 + 扫描状态），
 * 各自的查询与结果独立。扫描轮询只在「扫描中」时运转。
 *
 * 重构记录（汇总 §3 / 04 §S1）：
 * - 流式终值合并抽出到 `search/grep-merge.ts`（按 (path, lineNumber) 去重，
 *   取代旧 `length` 比较）；
 * - 轮询抽出到 `search/status-poller.ts`；
 * - 行为不变，单测可独立覆盖 helper。
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
  /** 文件名 IPC 失败原因（之前 catch 被吞，UI 永远"无结果"；汇总 §3.9）。 */
  filesError = $state('');
  /** 搜索模式（汇总 P1 严格度切换）：fuzzy / substring / regex。 */
  filesMode: SearchFilesMode = $state<SearchFilesMode>('fuzzy');

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

  private poller = new StatusPoller({
    intervalMs: 300,
    isActive: () => this.scanning,
    onTick: () => this.refreshStatus(),
  });

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
      this.poller.start();
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

  /** 文件名搜索（防抖由页面触发端调用 scheduleFiles）。 */
  async runFiles(): Promise<void> {
    const query = this.filesQuery.trim();
    if (!query) {
      this.filesResult = null;
      this.filesError = '';
      return;
    }
    this.filesBusy = true;
    this.filesError = '';
    // 快照当前 root generation：与 grepSeq 二选一对照，覆盖前端的代际语义
    // （汇总 §3.6 / PR3.5：根目录切换后旧结果视为过期，区别于 grepSeq 专管 grep）。
    const generationAtStart = this.status?.generation ?? null;
    const mode = this.filesMode;
    try {
      const page = await call<FilesPage>('search_files', {
        query,
        limit: 200,
        offset: 0,
        mode,
      });
      // 期间换根 → 静默丢弃结果（汇总 §3.6 第四种未覆盖场景）。
      if (this.status?.generation !== generationAtStart) return;
      this.filesResult = page;
    } catch (error) {
      // 之前 catch 被吞、UI 永远"无结果"；汇总 §3.9 提议统一 lastError。
      if (this.status?.generation !== generationAtStart) return;
      this.filesError = error instanceof Error ? error.message : String(error);
      this.filesResult = null;
    } finally {
      this.filesBusy = false;
    }
  }

  /**
   * 续页（汇总 P3）：基于已加载 items 总数 +1 取下一页。
   * 由 FilesPage.loadMore 调用；返回 items 不写回 filesResult（由
   * FilesPage 自己 accumulate），避免 store 多源真相。
   */
  async searchMoreFiles(
    query: string,
    _totalMatched: number,
    _alreadyLoaded: number,
  ): Promise<FileHit[]> {
    const trimmed = query.trim();
    if (!trimmed) return [];
    const generationAtStart = this.status?.generation ?? null;
    const offset = _alreadyLoaded;
    try {
      const page = await call<FilesPage>('search_files', {
        query: trimmed,
        limit: 200,
        offset,
        mode: this.filesMode,
      });
      if (this.status?.generation !== generationAtStart) return [];
      return page.items;
    } catch {
      return [];
    }
  }

  scheduleFiles(): void {
    if (this.filesDebounce) clearTimeout(this.filesDebounce);
    this.filesDebounce = setTimeout(() => void this.runFiles(), 80);
  }

  /**
   * 内容搜索（流式）：后端分片推进、经 Channel 逐批推送，本端即到即渲染。
   * 新查询使旧代际失效（分片与终值都被丢弃）；终值与通道累计按
   * `(path, lineNumber)` 去重并集（见 `grep-merge.ts`）。
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
    // 根目录切换的代际：open() 拉到的 status.generation 就是真相源（汇总 §3.6）。
    // 后端 grep 循环每分片比对 generation（commands.rs:444）会主动终止，前端
    // 仅在 finally / catch 兜底，避免 result 写回新 generation 的根。
    const generationAtStart = this.status?.generation ?? null;
    // 本任务运行期间是否检测到根目录切换（用于 finally 释放 grepBusy 状态）。
    let generationChanged = false;

    this.grepBusy = true;
    this.grepError = '';
    const chunks: GrepProgress[] = [];
    const flush = (): void => {
      // 先用空 final 占位（让列表进入"搜索中"态），最终 await 后用
      // mergeGrepStream 与终值做集合并。
      const merged = mergeGrepStream(chunks, undefined);
      this.grepResult = {
        items: merged.items,
        filesSearched: merged.filesSearched,
        filesWithMatches: merged.filesWithMatches,
        nextFileOffset: 0,
        aborted: false,
      };
    };
    flush();

    const onProgress = new Channel<GrepProgress>();
    onProgress.onmessage = (progress) => {
      if (seq !== this.grepSeq) return; // 过期分片丢弃。
      if (this.status?.generation !== generationAtStart) {
        // 根目录切换 → 后续所有分片连同最终结果一并丢弃。
        // grepSeq++ 让后续 onmessage 与 await 后置全部跳过（汇总 §3.6）。
        this.grepSeq += 1;
        generationChanged = true;
        return;
      }
      chunks.push(progress);
      flush();
    };

    try {
      const page = await call<GrepPage>('search_content', { query, opts, onProgress });
      if (seq !== this.grepSeq) return;
      if (this.status?.generation !== generationAtStart) {
        generationChanged = true;
        return;
      }
      // 终值 + 通道累计按 (path,lineNumber) 去重并集：终值若比通道累计少
      // 也不丢弃任何命中；终值带 nextFileOffset / aborted（通道不发）。
      const merged = mergeGrepStream(chunks, page);
      this.grepResult = {
        items: merged.items,
        filesSearched: merged.filesSearched,
        filesWithMatches: merged.filesWithMatches,
        nextFileOffset: merged.nextFileOffset,
        aborted: merged.aborted,
      };
    } catch (error) {
      if (seq === this.grepSeq && this.status?.generation === generationAtStart) {
        this.grepError = error instanceof Error ? error.message : String(error);
      }
    } finally {
      // grepBusy 复位规则（汇总 §3.6）：
      // 1. 本调用 seq 仍是当前 → 正常完成，复位。
      // 2. seq 被 generation 守卫 ++（旧任务被新根取代）→ 复位（本任务已结束）。
      // 3. seq 被新 runGrep ++（用户连发新查询）→ 不复位（busy 由新调用接管）。
      //    注意：这两种 seq++ 在 finally 看起来一样（都是 seq !== grepSeq），
      //    需要用 onProgress 里的局部标志区分（generationChanged）。
      if (seq === this.grepSeq) {
        this.grepBusy = false;
      } else if (generationChanged) {
        // 旧任务被根目录切换取代 → 释放本任务的 busy 状态；UI 立即恢复可交互。
        // 仅当没有更新的 runGrep 在跑（即本任务就是最后一次 grepSeq++ 的原因）。
        // 此处 generationChanged 是本任务期间检测到的信号，不会与新 runGrep 冲突
        // （新 runGrep 会进入自己的 runGrep()，busy=true 会重设）。
        this.grepBusy = false;
      }
    }
  }

  scheduleGrep(): void {
    if (this.grepDebounce) clearTimeout(this.grepDebounce);
    this.grepDebounce = setTimeout(() => void this.runGrep(), 200);
  }

  async cancelGrep(): Promise<void> {
    // 先让旧 promise 的 seq 守卫立即失配（避免旧帧 finally 写到 grepBusy）。
    // 然后调后端置 token=true；旧 promise 的 Channel onmessage 到达时
    // seq !== grepSeq 会被守卫丢弃。
    this.grepSeq += 1;
    await call('search_cancel');
  }

  dispose(): void {
    this.poller.stop();
    if (this.filesDebounce) clearTimeout(this.filesDebounce);
    if (this.grepDebounce) clearTimeout(this.grepDebounce);
    this.filesDebounce = null;
    this.grepDebounce = null;
  }
}

export const search = new SearchStore();
