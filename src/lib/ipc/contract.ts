/**
 * IPC 合同的唯一来源（架构文档 §5）。
 *
 * 规则：
 * - 命令名与 Rust 侧 `#[tauri::command]` 一一对应，由 `contract.test.ts`
 *   解析 Rust 源码比对锁死，不一致即 `pnpm check` 失败；
 * - 前端任何文件禁止绕过本文件直接 `invoke`（编码规范 §7）；
 * - 入参/返回类型必须与 Rust 侧 serde 结构保持字段级一致。
 */

/** 全部 IPC 命令。新增命令时在这里登记，Rust 侧同名实现。 */
export const IPC_COMMANDS = [
  'app_meta',
  'app_toggle_devtools',
  'settings_get',
  'settings_update',
  'harness_environment',
  'harness_status',
  'harness_proxy_url',
  'harness_start',
  'harness_stop',
  'harness_install',
  'harness_install_node',
  'harness_log',
  'search_open',
  'search_status',
  'search_files',
  'search_content',
  'search_cancel',
  'search_wait_ready',
  'search_list_drives',
  'shots_capture',
  'shots_overlay_ready',
  'shots_set_hotkey',
  'shots_clear_hotkey',
  'shots_copy_clipboard',
  'shots_save',
  'shots_pin',
  'shots_close_overlays',
  'shots_open_pin',
  'terminal_spawn',
  'terminal_write',
  'terminal_resize',
  'terminal_kill',
  'terminal_replay',
  'terminal_clear',
  'terminal_pin',
  'terminal_unpin',
  'terminal_pin_resume',
  'terminal_pinned_list',
  'terminal_pinned_replay',
  'notes_list',
  'notes_read',
  'notes_save',
  'notes_create',
  'notes_delete',
  'notes_init',
  'bridge_deploy',
  'bridge_status',
  'remote_interfaces',
  'remote_status',
  'remote_pair',
  'remote_revoke',
  'remote_self_check',
  'sync_status',
  'sync_init',
  'sync_pull',
  'sync_push',
] as const;

export type IpcCommand = (typeof IPC_COMMANDS)[number];

/** 全部事件通道（Rust 侧 emit ↔ 前端 listen）。 */
export const IPC_EVENTS = [
  'harness://event',
  'harness://install-progress',
  'terminal://output',
  'terminal://exit',
] as const;

// ---------------------------------------------------------------------------
// app_meta
// ---------------------------------------------------------------------------

export interface AppMetaResult {
  name: string;
  version: string;
  identifier: string;
}

// ---------------------------------------------------------------------------
// settings_get / settings_update
// ---------------------------------------------------------------------------

export type ThemePreference = 'system' | 'light' | 'dark';

export interface WindowGeometry {
  x: number;
  y: number;
  width: number;
  height: number;
  maximized: boolean;
}

export interface WindowSettings {
  closeToTray: boolean;
  startMinimized: boolean;
  geometry: WindowGeometry | null;
}

export type DshVersionStrategy = 'pinned' | 'existing';

export type DshHomePolicy = 'isolated' | 'system';

export interface DshSettings {
  port: number;
  allowRandomFallback: boolean;
  versionStrategy: DshVersionStrategy;
  autostart: boolean;
  home: DshHomePolicy;
}

export interface MirrorsSettings {
  nodeBinary: 'auto' | 'official' | 'npmmirror';
  npmRegistry: string;
}

export interface SearchSettings {
  rootHistory: string[];
}

export interface Settings {
  schemaVersion: number;
  theme: ThemePreference;
  window: WindowSettings;
  dsh: DshSettings;
  mirrors: MirrorsSettings;
  search: SearchSettings;
  hotkeys: HotkeysSettings;
  terminal: TerminalSettings;
  notes: NotesSettings;
  remote: RemoteSettings;
}

/**
 * settings_update 的入参：字段级部分更新。
 * schemaVersion 与 window.geometry 由 Rust 侧独占管理，前端不可通过补丁修改。
 */
export interface SettingsPatch {
  theme?: ThemePreference;
  window?: Partial<Pick<WindowSettings, 'closeToTray' | 'startMinimized'>>;
  dsh?: Partial<Omit<DshSettings, never>>;
  mirrors?: Partial<MirrorsSettings>;
  search?: Partial<SearchSettings>;
  hotkeys?: Partial<HotkeysSettings>;
  terminal?: Partial<TerminalSettings>;
  notes?: Partial<NotesSettings>;
  remote?: Partial<RemoteSettings>;
}

// ---------------------------------------------------------------------------
// harness_*（DSH 托管域）
// ---------------------------------------------------------------------------

/** `node --version` 的三段号（Rust node-runtime::Version）。 */
export interface NodeVersion {
  major: number;
  minor: number;
  patch: number;
}

export type NodeSource = 'path' | 'nvm' | 'fnm' | 'volta' | 'system' | 'managed';

export interface NodeInstallation {
  path: string;
  version: NodeVersion;
  source: NodeSource;
}

export function formatNodeVersion(version: NodeVersion): string {
  return `v${version.major}.${version.minor}.${version.patch}`;
}

export interface HarnessEnvironment {
  node: NodeInstallation | null;
  allNodeRuntimes: NodeInstallation[];
  minimumNode: NodeVersion;
  dshInstalled: boolean;
  dshVersion: string | null;
  installSpec: string;
  dshEntry: string;
  workspace: string;
  dshHome: string;
  /** 一键安装将下载的 Node 版本（Rust 侧单一事实源，避免前端硬编码）。 */
  bundledNodeVersion: string;
}

/** supervisor 状态机（serde tag = "phase"）。 */
export type HarnessStatus =
  | { phase: 'stopped' }
  | { phase: 'starting' }
  | {
      phase: 'ready';
      /** scheme://host:port，去 token 的干净 origin（展示、托盘等用）。 */
      origin: string;
      /**
       * 含 `?token=` 的完整 URL。仅供「在系统浏览器打开」（顶层导航不受
       * SameSite 限制）；DSH 页 iframe 走 `harness_proxy_url` 的回环代理
       * （Strict cookie 在跨站 iframe 不可携带，cookie 由服务端持有）。
       */
      url: string;
      pid: number;
    }
  | { phase: 'restarting'; attempt: number; delayMs: number }
  | { phase: 'failed'; reason: string };

/** harness_proxy_url 返回：DSH 回环代理地址（http://127.0.0.1:<port>）；未启动 = null。 */
export type HarnessProxyUrlResult = string | null;

export type HarnessStream = 'stdout' | 'stderr';

export interface HarnessLogLine {
  stream: HarnessStream;
  line: string;
}

/** harness://event 事件负载（serde tag = "kind"）。 */
export type HarnessEvent =
  ({ kind: 'status' } & HarnessStatus) | { kind: 'log'; stream: HarnessStream; line: string };

export function formatHarnessStatus(status: HarnessStatus): string {
  switch (status.phase) {
    case 'stopped':
      return '未运行';
    case 'starting':
      return '启动中…';
    case 'ready':
      return `运行中 · ${status.origin}`;
    case 'restarting':
      return `重启中（第 ${status.attempt} 次，${status.delayMs}ms 后）`;
    case 'failed':
      return `启动失败：${status.reason}`;
  }
}

/**
 * harness://install-progress 事件负载（serde tag = "stage"）。
 * Node 下载带字节级进度；DSH 走 pnpm，只有包数推进。
 * 可选数值字段为 null 表示该维度暂无数据，展示层沿用上一个事件。
 */
export type InstallProgress =
  | { stage: 'node-manifest'; source: string }
  | {
      stage: 'node-download';
      source: string;
      url: string;
      totalBytes: number | null;
      downloadedBytes: number;
    }
  | { stage: 'node-finalize'; source: string; activity: string }
  | {
      stage: 'dsh-packages';
      registry: string;
      resolved: number | null;
      downloaded: number;
      added: number;
      totalHint: number | null;
    }
  | { stage: 'done' };

// ---------------------------------------------------------------------------
// search_*（搜索域，M2）
// ---------------------------------------------------------------------------

export interface SearchOpen {
  root: string;
  generation: number;
  rebuilt: boolean;
}

export interface SearchStatus {
  root: string | null;
  generation: number;
  scanning: boolean;
  watcherReady: boolean;
  files: number;
}

export interface FileHit {
  path: string;
  score: number;
  /** 文件名内的匹配区间（字节偏移，UTF-8 切片高亮）。 */
  offsets: Array<[number, number]>;
  /** 大小（字节）与修改时间（毫秒）——结果表排序列；stat 失败记 0。 */
  size: number;
  mtime: number;
}

export interface FilesPage {
  items: FileHit[];
  totalMatched: number;
  totalFiles: number;
}

export interface GrepHit {
  path: string;
  lineNumber: number;
  col: number;
  lineContent: string;
  /** 行内匹配区间（字节偏移）。 */
  offsets: Array<[number, number]>;
  contextBefore: string[];
  contextAfter: string[];
}

export interface GrepOptions {
  regex: boolean;
  smartCase: boolean;
  beforeContext: number;
  afterContext: number;
  /** 文件名 glob 过滤（`*.rs` 按文件名；含 `/` 按相对路径）。空 = 不过滤。 */
  glob?: string;
}

export interface GrepPage {
  items: GrepHit[];
  filesSearched: number;
  filesWithMatches: number;
  /** 流式循环下非 0 仅表示「已中断」，前端不再手动翻页。 */
  nextFileOffset: number;
  aborted: boolean;
}

/** search_content 的流式分片（Tauri Channel 推送）。 */
export interface GrepProgress {
  items: GrepHit[];
  filesSearched: number;
  filesWithMatches: number;
}

/** 一个逻辑盘（search_list_drives 返回项，搜索根选择器）。 */
export interface DriveInfo {
  /** 形如 `C:\` 的根路径。 */
  path: string;
  /** fixed | removable | network | cdrom | ramdisk */
  kind: string;
  totalBytes: number;
  freeBytes: number;
}

// ---------------------------------------------------------------------------
// shots_*（截屏域，M3）
// ---------------------------------------------------------------------------

export interface HotkeysSettings {
  /** Tauri 快捷键语法（如 "Alt+A"）；空串 = 不注册。 */
  screenshot: string;
}

export interface FrozenMonitor {
  index: number;
  /** 虚拟屏幕坐标（物理像素）。 */
  x: number;
  y: number;
  width: number;
  height: number;
  /// DPI 缩放（物理→逻辑换算用）。
  scale: number;
  /** 底图 PNG 路径（convertFileSrc 引用）。 */
  image: string;
}

// ---------------------------------------------------------------------------
// terminal_*（终端域，M4）
// ---------------------------------------------------------------------------

/** `terminal_spawn` 的返回：会话 id + 实际解析出的 shell（标签默认标题用）。 */
export interface TerminalInfo {
  id: number;
  shell: string;
}

export interface TerminalOutputEvent {
  id: number;
  data: string;
}

export interface TerminalExitEvent {
  id: number;
  exitCode: number | null;
}

export interface TerminalSettings {
  /** auto = pwsh 优先、powershell 兜底；否则为可执行文件路径。 */
  shell: string;
  fontSize: number;
  scrollback: number;
}

/** terminal_pinned_list 的返回项：一条固定（PIN）终端的元数据。 */
export interface PinnedTerminal {
  pinId: number;
  title: string;
  shell: string;
  cwd: string | null;
}

// ---------------------------------------------------------------------------
// notes_*（笔记域，M5）
// ---------------------------------------------------------------------------

export interface NoteMeta {
  /** 相对于库目录的路径（正斜杠）。 */
  path: string;
  title: string;
  tags: string[];
  /** 正文首行摘要（跳过标题行，截断 80 字符）。 */
  excerpt: string;
  /** 文件修改时间（毫秒时间戳）。 */
  updated: number;
  size: number;
}

export interface NoteContent {
  meta: NoteMeta;
  body: string;
}

export interface NotesSettings {
  /** 库目录绝对路径；空串 = 尚未初始化。 */
  vaultDir: string;
}

// ---------------------------------------------------------------------------
// bridge_*（DSH 插件桥域，M6）
// ---------------------------------------------------------------------------

export interface BridgeStatus {
  /** 插件文件已在 profile node_modules 就位。 */
  deployed: boolean;
  /** cordis.patch.yml 已含 qx-bridge 条目。 */
  patchEntry: boolean;
  /** patch 配置里的 vault 与当前设置一致。 */
  vaultMatch: boolean;
  vaultDir: string;
  pluginDir: string;
  profileDir: string;
  /** DSH 进程当前是否在跑（跑着则需重启才加载桥）。 */
  dshRunning: boolean;
}

// ---------------------------------------------------------------------------
// remote_*（远程网关域，R1）
// ---------------------------------------------------------------------------

export interface RemoteDevice {
  id: string;
  name: string;
  token: string;
  createdAt: number;
  revoked: boolean;
}

export interface RemoteSettings {
  enabled: boolean;
  /** 绑定 IP（本机网卡地址；EasyTier 网卡地址即可）。 */
  bindIp: string;
  port: number;
  devices: RemoteDevice[];
}

export interface NetInterface {
  name: string;
  ip: string;
  easytier: boolean;
}

export interface RemoteStatus {
  enabled: boolean;
  bindIp: string;
  port: number;
  /** 监听地址（未监听 = null）。 */
  listening: string | null;
  deviceCount: number;
  activeCount: number;
  dshRunning: boolean;
}

/** remote_self_check 返回：网关健康自检（带真实 token 走 /qx-gate）。 */
export interface SelfCheck {
  ok: boolean;
  detail: string;
  latencyMs: number;
}

// ---------------------------------------------------------------------------
// sync_*（同步域，S1 第一阶段）
// ---------------------------------------------------------------------------

export interface SyncStatus {
  gitAvailable: boolean;
  initialized: boolean;
  hasRemote: boolean;
  dirty: number;
  ahead: number | null;
  behind: number | null;
  vault: string;
}

/**
 * 字节偏移区间切分：Rust 侧给的是 UTF-8 字节偏移，这里换算成
 * JS 字符下标供 slice/高亮使用。
 */
export function sliceByByteOffsets(
  text: string,
  offsets: Array<[number, number]>,
): Array<{
  text: string;
  matched: boolean;
}> {
  // 先构建字节→字符映射（仅按需扫描一次）。
  const byteToChar = new Map<number, number>();
  let bytes = 0;
  for (let index = 0; index < text.length; index++) {
    byteToChar.set(bytes, index);
    // surrogate pair 算 2 个 char 但 4 字节；统一按 code point 展开。
    const code = text.codePointAt(index) ?? 0;
    bytes += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
    if (code > 0xffff) index++;
  }
  byteToChar.set(bytes, text.length);

  const segments: Array<{ text: string; matched: boolean }> = [];
  let cursor = 0;
  for (const [start, end] of offsets) {
    const charStart = byteToChar.get(start) ?? text.length;
    const charEnd = byteToChar.get(end) ?? text.length;
    if (charStart > cursor) {
      segments.push({ text: text.slice(cursor, charStart), matched: false });
    }
    if (charEnd > charStart) {
      segments.push({ text: text.slice(charStart, charEnd), matched: true });
    }
    cursor = Math.max(cursor, charEnd);
  }
  if (cursor < text.length) {
    segments.push({ text: text.slice(cursor), matched: false });
  }
  return segments.length > 0 ? segments : [{ text, matched: false }];
}
