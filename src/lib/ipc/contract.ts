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
  'settings_get',
  'settings_update',
  'harness_environment',
  'harness_status',
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
  'shots_capture',
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
  'terminal_list',
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
  'sync_status',
  'sync_init',
  'sync_pull',
  'sync_push',
] as const;

export type IpcCommand = (typeof IPC_COMMANDS)[number];

/** 全部事件通道（Rust 侧 emit ↔ 前端 listen）。 */
export const IPC_EVENTS = ['harness://event', 'terminal://output', 'terminal://exit'] as const;

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
  pinnedVersion: string;
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
}

/** supervisor 状态机（serde tag = "phase"）。 */
export type HarnessStatus =
  | { phase: 'stopped' }
  | { phase: 'starting' }
  | { phase: 'ready'; origin: string; pid: number }
  | { phase: 'restarting'; attempt: number; delayMs: number }
  | { phase: 'failed'; reason: string };

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
}

export interface GrepPage {
  items: GrepHit[];
  filesSearched: number;
  filesWithMatches: number;
  nextFileOffset: number;
  aborted: boolean;
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

// ---------------------------------------------------------------------------
// notes_*（笔记域，M5）
// ---------------------------------------------------------------------------

export interface NoteMeta {
  /** 相对于库目录的路径（正斜杠）。 */
  path: string;
  title: string;
  tags: string[];
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
