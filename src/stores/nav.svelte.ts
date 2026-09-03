export type PageId =
  | 'overview'
  | 'env'
  | 'console'
  | 'dsh'
  | 'search-files'
  | 'search-grep'
  | 'terminal'
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

  go(page: PageId): void {
    this.page = page;
  }

  /** 标记某页需要挂载（用户进入时，或后台预热如 DSH iframe）。 */
  visit(page: PageId): void {
    this.visited[page] = true;
  }
}

export const nav = new NavStore();
