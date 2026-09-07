export type PageId =
  | 'overview'
  | 'env'
  | 'dsh'
  | 'search-files'
  | 'search-grep'
  | 'terminal'
  | 'remote'
  | 'notes'
  | 'settings';

/** 外壳导航域。功能域随里程碑各自追加（终端/截屏/笔记…）。 */
class NavStore {
  page: PageId = $state('overview');
  /**
   * keep-alive：已首次挂载的页面。切页只做显隐，组件与状态永不销毁；
   * 没进过的页面不挂载（终端不空起 PTY，DSH 不空载 iframe）。
   */
  visited: Partial<Record<PageId, boolean>> = $state({ overview: true });
  /**
   * 已分离到独立窗口的页面：主窗侧栏与页面容器同时让位，
   * 独立窗口关闭（window://closed）后自动恢复。
   */
  detached: Partial<Record<PageId, boolean>> = $state({});

  go(page: PageId): void {
    this.page = page;
  }

  /** 标记某页需要挂载（用户进入时，或后台预热如 DSH iframe）。 */
  visit(page: PageId): void {
    this.visited[page] = true;
  }

  /** 页面已分离：侧栏项与主窗容器一并让位。 */
  detach(page: PageId): void {
    this.detached[page] = true;
    if (this.page === page) this.page = 'overview';
  }

  /** 独立窗口关闭：恢复侧栏项与主窗容器。 */
  reattach(page: PageId): void {
    delete this.detached[page];
  }
}

export const nav = new NavStore();
