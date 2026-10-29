import { listen } from '@tauri-apps/api/event';
import { call } from '../lib/ipc';
import type {
  IntelFavorite,
  IntelSaveArgs,
  IntelTopicDay,
  IntelTopicStatus,
  IntelUpdatedEvent,
  FeedItem,
  Article,
  ArticlesDay,
  DigestProgress,
  IntelFetchingEvent,
} from '../lib/ipc/contract';

/** 状态页里单主题的聚合行（docs/17 §7.4「进度可见」）。 */
export interface StatusRow {
  topic: IntelTopicStatus;
  /** 出刊产物（Feed 主题；null = 模型主题或未拉取）。 */
  articles: ArticlesDay | null;
  /** 出刊进度。 */
  digest: DigestProgress | null;
}

/**
 * 情报域状态（docs/17）。数据源是 Rust intel 模块的按天落盘文件，
 * 前端只读文件 + 触发刷新；`intel://updated` / `intel://digest` 事件驱动
 * 状态、当天视图与出刊进度刷新。
 */

/** Feed 主题内视图：出刊文章流（默认）/ 原始源列表。 */
export type FeedTab = 'digest' | 'raw';

/** 源徽章显示名（sourceId → 短标签）。 */
const SOURCE_LABELS: Record<string, string> = {
  arxiv: 'arXiv',
  'hf-papers': 'HF Papers',
  'reddit-ml': 'r/ML',
  'reddit-localllama': 'r/LocalLLaMA',
  qbitai: '量子位',
  jiqizhixin: '机器之心',
  'hf-blog': 'HF Blog',
  'the-decoder': 'The Decoder',
  solidot: 'Solidot',
  hn: 'HN',
};

export function sourceLabel(sourceId: string): string {
  return SOURCE_LABELS[sourceId] ?? sourceId;
}

/** 出刊阶段顺序（状态弹框时间线，docs/17 §7.4）。 */
export const DIGEST_STAGES = ['fetching', 'triage', 'bodies', 'rewriting', 'synthesis'] as const;

export function stageLabel(stage: string): string {
  switch (stage) {
    case 'fetching':
      return '抓取源数据';
    case 'triage':
      return '筛选';
    case 'bodies':
      return '取正文';
    case 'rewriting':
      return '深加工';
    case 'synthesis':
      return '归并';
    case 'done':
      return '出刊完成';
    case 'failed':
      return '出刊失败';
    case 'llm-unavailable':
      return 'LLM 通道不可用';
    default:
      return stage;
  }
}

class IntelStore {
  /** 主题状态（卡片网格数据源）。 */
  topics = $state<IntelTopicStatus[]>([]);
  favorites = $state<IntelFavorite[]>([]);
  /** null = 卡片网格；'favorites' = 收藏；其余 = 主题 id。 */
  active = $state<string | null>(null);
  /** 查看日期（'' = 今天）。 */
  date = $state('');
  day = $state<IntelTopicDay | null>(null);
  /** 该主题有数据的日期（降序）。 */
  days = $state<string[]>([]);
  loading = $state(false);
  /** 手动刷新进行中（本地回显，权威态在 topics[].fetching）。 */
  refreshing = $state(false);
  error = $state('');
  /** Feed 主题内 tab：出刊文章流 / 原始源。 */
  feedTab = $state<FeedTab>('digest');
  /** 当天出刊（articles 空 = 尚未出刊）。 */
  articles = $state<ArticlesDay | null>(null);
  /** 出刊进度（当前主题；事件实时推送）。 */
  digest = $state<DigestProgress | null>(null);
  /** 阅读视图打开的文章。 */
  reading = $state<Article | null>(null);
  /** 全局状态页显隐（点击「状态」按钮打开）。 */
  statusOpen = $state(false);
  /** 状态页数据：每主题聚合行。 */
  statusRows = $state<StatusRow[]>([]);
  /** DSH 运行阶段（打开状态页时拉取）。 */
  dshPhase = $state('');
  private wired = false;

  topic(id: string | null): IntelTopicStatus | undefined {
    if (id === null || id === 'favorites') return undefined;
    return this.topics.find((t) => t.id === id);
  }

  /** 全部主题里最近一次成功抓取（入口头部「HH:mm 更新」）。 */
  get lastFetchAt(): number | null {
    let latest: number | null = null;
    for (const t of this.topics) {
      if (t.lastFetchAt !== null && (latest === null || t.lastFetchAt > latest)) {
        latest = t.lastFetchAt;
      }
    }
    return latest;
  }

  /** 当前主题是否杂志流（M3 起所有主题都是；保留语义位给收藏视图）。 */
  get activeIsFeed(): boolean {
    return this.active !== null && this.active !== 'favorites';
  }

  saved(id: string): boolean {
    return this.favorites.some((f) => f.id === id);
  }

  /** 出刊进行中（横幅判断，当前主题）。 */
  get digestRunning(): boolean {
    return this.digest !== null && this.digestRunningOf(this.digest);
  }

  /** 任意出刊进度是否进行中。 */
  digestRunningOf(d: DigestProgress): boolean {
    return DIGEST_STAGES.includes(d.stage as (typeof DIGEST_STAGES)[number]);
  }

  /** 事件订阅 + 首载。IntelPage onMount 调一次（keep-alive 下不重复）。 */
  async wire(): Promise<void> {
    if (this.wired) return;
    this.wired = true;
    await listen<IntelUpdatedEvent>('intel://updated', (event) => {
      void this.onUpdated(event.payload.topicId);
    });
    await listen<DigestProgress>('intel://digest', (event) => {
      // 横幅数据（当前主题）。
      if (event.payload.topicId === this.active) {
        this.digest = event.payload;
        // 出刊完成 → 当前主题文章流自动换成新刊。
        if (event.payload.stage === 'done') void this.loadArticles();
      }
      // 状态页行（打开时实时刷新对应主题行）。
      const row = this.statusRows.find((r) => r.topic.id === event.payload.topicId);
      if (row) {
        row.digest = event.payload;
        if (event.payload.stage === 'done') {
          void call<ArticlesDay>('intel_articles', { topicId: row.topic.id })
            .then((day) => (row.articles = day))
            .catch(() => {});
          // 要览随新刊刷新。
          if (this.frontLoaded) void this.loadFrontPage();
        }
      }
    });
    await listen<IntelFetchingEvent>('intel://fetching', (event) => {
      // topics 与 statusRows 共享同一主题对象引用：更新一处即两处可见。
      const topic = this.topics.find((t) => t.id === event.payload.topicId);
      if (topic) topic.active = event.payload.active;
    });
    await this.load();
    // 首载要览（wire 只跑一次；出刊 done 事件里增量刷新）。
    void this.loadFrontPage();
  }

  async load(): Promise<void> {
    try {
      const [topics, favorites] = await Promise.all([
        call<IntelTopicStatus[]>('intel_topics'),
        call<IntelFavorite[]>('intel_favorites'),
      ]);
      this.topics = topics;
      this.favorites = favorites;
    } catch (cause) {
      this.error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  private async onUpdated(topicId: string): Promise<void> {
    this.refreshing = false;
    await this.load();
    if (this.active === topicId) await this.loadDay();
  }

  /** 今日要览行（docs/17 §7.5.2-1）：每 Feed 主题的当日/最近一期文章。 */
  frontRows = $state<{ topicId: string; label: string; day: ArticlesDay | null }[]>([]);
  private frontLoaded = false;

  /** 聚合全部主题的出刊（wire 后 + 出刊完成后刷新）。 */
  async loadFrontPage(): Promise<void> {
    const rows = await Promise.all(
      this.topics.map(async (topic) => ({
        topicId: topic.id,
        label: topic.label,
        day: await call<ArticlesDay>('intel_articles', { topicId: topic.id }).catch(() => null),
      })),
    );
    this.frontRows = rows.filter((r) => r.day !== null && r.day.articles.length > 0);
    this.frontLoaded = true;
  }

  /** 打开要览/收藏里的文章：先进主题（加载上下文）再进阅读视图。 */
  async openArticle(article: Article, topicId: string): Promise<void> {
    await this.open(topicId);
    this.reading = article;
  }

  /** 进入主题 / 收藏视图。 */
  async open(id: string): Promise<void> {
    this.active = id;
    this.date = '';
    this.day = null;
    this.feedTab = 'digest';
    this.reading = null;
    this.digest = null;
    await this.loadDays();
    await this.loadDay();
    await this.loadArticles();
    try {
      this.digest = await call<DigestProgress | null>('intel_digest_status', { topicId: id });
    } catch {
      this.digest = null;
    }
  }

  back(): void {
    this.active = null;
    this.day = null;
    this.reading = null;
    this.digest = null;
  }

  /**
   * 打开全局状态页：聚合全部主题（抓取 + 出刊）与 DSH 通道。
   * 打开后 intel://digest 事件实时刷新对应行。
   */
  async openStatus(): Promise<void> {
    this.statusOpen = true;
    const [phase, ...rows] = await Promise.all([
      call<{ phase: string }>('harness_status')
        .then((s) => s.phase)
        .catch(() => 'unknown'),
      ...this.topics.map(async (topic) => {
        const [articles, digest] = await Promise.all([
          call<ArticlesDay>('intel_articles', { topicId: topic.id }).catch(() => null),
          call<DigestProgress | null>('intel_digest_status', { topicId: topic.id }).catch(
            () => null,
          ),
        ]);
        return { topic, articles, digest } as StatusRow;
      }),
    ]);
    this.dshPhase = phase;
    this.statusRows = rows.sort(
      (a, b) =>
        this.topics.findIndex((t) => t.id === a.topic.id) -
        this.topics.findIndex((t) => t.id === b.topic.id),
    );
  }

  closeStatus(): void {
    this.statusOpen = false;
  }

  async loadDays(): Promise<void> {
    if (this.active === null || this.active === 'favorites') return;
    this.days = await call<string[]>('intel_days', { topicId: this.active });
  }

  async loadDay(): Promise<void> {
    if (this.active === null || this.active === 'favorites') return;
    const topicId = this.active;
    this.loading = true;
    this.error = '';
    try {
      this.day = await call<IntelTopicDay>('intel_topic_day', {
        topicId,
        date: this.date || undefined,
      });
    } catch (cause) {
      this.error = cause instanceof Error ? cause.message : String(cause);
      this.day = null;
    } finally {
      this.loading = false;
    }
  }

  async loadArticles(): Promise<void> {
    if (this.active === null || this.active === 'favorites') return;
    try {
      this.articles = await call<ArticlesDay>('intel_articles', {
        topicId: this.active,
        date: this.date || undefined,
      });
    } catch {
      this.articles = null;
    }
  }

  /** 当前展示的是往期回退（articles.date ≠ 本地今天）：显示「{date} 刊」徽章。 */
  get articlesFallback(): boolean {
    if (this.articles === null || this.articles.articles.length === 0) return false;
    const now = new Date();
    const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
    return this.articles.date !== today;
  }

  /** 手动出刊（重跑当天加工；进度经 intel://digest 推送 + 状态页可见）。 */
  async startDigest(topicId?: string): Promise<void> {
    const id = topicId ?? this.active;
    if (id === null || id === undefined || id === 'favorites') return;
    try {
      await call('intel_digest_start', { topicId: id });
    } catch (cause) {
      this.error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  /** 手动刷新当前主题（重抓 + 自动重出刊，结果经事件回推）。 */
  async refresh(): Promise<void> {
    if (this.active === null || this.active === 'favorites') return;
    this.refreshing = true;
    try {
      await call('intel_refresh', { topicId: this.active });
    } catch (cause) {
      this.refreshing = false;
      this.error = cause instanceof Error ? cause.message : String(cause);
    }
  }

  async changeDate(date: string): Promise<void> {
    this.date = date;
    this.reading = null;
    await this.loadDay();
    await this.loadArticles();
  }

  /** 日期导航：取 days 里相邻的日期（'' = 今天，视为最前）。 */
  get canGoOlder(): boolean {
    const current = this.date || (this.days[0] ?? '');
    return this.days.some((d) => d < current);
  }

  get canGoNewer(): boolean {
    return this.date !== '';
  }

  async goOlder(): Promise<void> {
    const current = this.date || (this.days[0] ?? '');
    const older = [...this.days].reverse().find((d) => d < current);
    if (older) await this.changeDate(older);
  }

  async goNewer(): Promise<void> {
    if (this.date === '') return;
    const newer = this.days.find((d) => d > this.date);
    await this.changeDate(newer ?? '');
  }

  async toggleSaveFeed(item: FeedItem): Promise<void> {
    await this.toggleSave({ topicId: item.sourceId, feed: item });
  }

  async toggleSaveArticle(article: Article): Promise<void> {
    const topicId =
      this.active === 'favorites' ? article.topicId : (this.active ?? article.topicId);
    await this.toggleSave({ topicId, article });
  }

  async toggleSave(args: IntelSaveArgs): Promise<void> {
    const id = args.feed?.id ?? args.article?.id;
    if (!id) return;
    if (this.saved(id)) {
      this.favorites = await call<IntelFavorite[]>('intel_unsave', { id });
    } else {
      const topicId = args.topicId || this.active || '';
      this.favorites = await call<IntelFavorite[]>('intel_save', {
        item: { ...args, topicId },
      });
    }
  }

  async unsave(id: string): Promise<void> {
    this.favorites = await call<IntelFavorite[]>('intel_unsave', { id });
  }

  /** 标已读：先本地回显（标题即刻置灰），落盘异步。 */
  markRead(ids: string[]): void {
    if (ids.length === 0 || !this.day) return;
    const now = Date.now();
    const read = { ...this.day.read };
    for (const id of ids) read[id] = now;
    this.day = { ...this.day, read };
    void call('intel_mark_read', { ids });
  }
}

export const intel = new IntelStore();
