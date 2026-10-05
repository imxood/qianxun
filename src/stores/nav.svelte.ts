export type PageId =
  'env' | 'dsh' | 'files' | 'web' | 'intel' | 'remote' | 'notes' | 'games' | 'plugins' | 'settings';

/** 外壳导航域。功能域随里程碑各自追加（截屏/笔记…）。 */
class NavStore {
  page: PageId = $state('env');
  /**
   * keep-alive：已首次挂载的页面。切页只做显隐，组件与状态永不销毁；
   * 没进过的页面不挂载（DSH 不空载 iframe）。
   */
  visited: Partial<Record<PageId, boolean>> = $state({ env: true });
  /**
   * 已分离到独立窗口的页面：主窗侧栏与页面容器同时让位，
   * 最后一个独立窗口关闭（window://closed）后自动恢复。
   *
   * **计数而非布尔**（分析 S6/P1）：布尔下重复分离（同一 view 开两个窗）
   * 会让任一窗口关闭都清空标记，主窗随后重新挂载一个**活的** DSH iframe
   * —— 同一 view 有两个活实例（一个在独立窗、一个在主窗），DSH 会话分裂。
   * 改为引用计数后，只有计数归零才恢复。
   */
  detached: Partial<Record<PageId, number>> = $state({});

  go(page: PageId): void {
    this.page = page;
  }

  /** 标记某页需要挂载（用户进入时，或后台预热如 DSH iframe）。 */
  visit(page: PageId): void {
    this.visited[page] = true;
  }

  /** 页面已分离：侧栏项与主窗容器一并让位（可多次，对应多个独立窗）。 */
  detach(page: PageId): void {
    this.detached[page] = (this.detached[page] ?? 0) + 1;
    if (this.page === page) this.page = 'env';
  }

  /**
   * 独立窗口关闭：递减该 view 的分离计数，归零才恢复侧栏项与主窗容器。
   * 未登记的关闭事件（计数已是 0）不产生负数，直接忽略。
   */
  reattach(page: PageId): void {
    const count = this.detached[page] ?? 0;
    if (count <= 1) delete this.detached[page];
    else this.detached[page] = count - 1;
  }
}

export const nav = new NavStore();
