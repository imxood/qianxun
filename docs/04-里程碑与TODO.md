# 04 · 里程碑与 TODO

状态：草案 v1（待确认）。每个里程碑 = 一个可发布版本；完成一项勾一项，本文件随开发推进更新。

## M0 · 工程地基（→ v0.1，约 1 周）

**目标**：千寻窗口能打开，质量门禁就位，移植底座就位。

- [x] 仓库初始化：Tauri 2 + Svelte 5 + Vite + TS strict + Tailwind 4 + vitest 模板跑通
- [x] 初始化私有 git 仓库
- [x] 质量门禁脚本：`pnpm check`、`cargo check`（fmt/clippy -D warnings/test）跑通并作为合入条件
- [x] 合同比对测试骨架：`lib/ipc/contract.ts` + Rust 命令清单比对测试
- [x] 移植底座：`paths` / `error` / `logging` / `atomic` / `window` / `tray` / `single_instance`
- [x] 设置系统：settings.json 读写（原子写 + schemaVersion）+ 设置 IPC + 空设置页
- [x] 外壳骨架：标题栏 + 侧栏导航（空页占位）+ 主题（深/浅/跟随系统）+ 状态栏
- [x] 应用标识与命名：`com.qianxun.app`、窗口标题"千寻"

**验收**：`pnpm tauri dev` 打开千寻窗口，导航/主题/托盘可用，门禁全绿。

## M1 · 核心托管（v0.1）✅ 已完成

**目标**：检测 → 安装 → 固定端口启动 → 守护 → 承载 DSH，全链路可用。

- [x] 移植 `proc-guard`、`node-runtime` 两个 crate（自有化，测试全保留）
- [x] 移植 supervisor 并**改造为固定端口**：`--port <配置>`、就绪行校验端口一致性、
      冲突显式报错（含保留端口 10000 专门提示）、健康探测、退避重启
- [x] **DSH_HOME 隔离**（ADR-009）：spawn 时注入 `DSH_HOME=<数据目录>/dsh-home`
- [x] DSH 安装：私有 prefix **pnpm 安装**（自带 pnpm@11.7.0，备份→直装→校验事务，
      registry 按 `mirrors.npmRegistry` 默认 npmmirror，allowBuilds 放行原生构建）
- [x] Node 安装：系统 curl.exe 下载官方 win-x64 zip + SHASUMS256 校验 +
      解压到 `node/`（auto=官方优先失败转 npmmirror），不动系统 PATH
- [x] 环境页 UI：Node/DSH 检测行 + "检测不到即安装按钮" + 安装进度实时日志
- [x] 控制台页：DSH 状态卡 + 手动启动/停止 + 日志流（回填 + 自动滚动）
- [x] 承载页：iframe 加载 DSH + 端口跟随 + 断线/重启自动重载
- [x] 设置页：端口、DSH HOME（isolated/system）、锁定版本、镜像源、`autostart`、随机回退
- [x] 托盘：显示运行状态、快捷启停
- [x] 冒烟验收（2026-09-03 全过）：17300 自启全链路；强杀 DSH 退避自愈（换 PID 保端口）；
      强杀千寻无孤儿（Job Object 连带回收）；10000 端口上的外部实例全程无扰；
      npm peer 求解爆炸的教训已固化为 pnpm 路线（§4.4）

**验收**：在一台"裸机"（无 Node）上，仅靠千寻完成 Node→DSH 安装并稳定承载，重启/崩溃自愈。
——本机（有 Node 24）已验证完整链路；裸机全链路待真实新机验证（Node 下载链路有真实网络测试背书）。

## M2 · 搜索（v0.1，2026-09-03 完成）

**目标**：文件名搜索与内容搜索两个独立页面，专业级交互与性能。

- [x] 确认 [fff](https://github.com/dmtrKovalenko/fff)（`fff-search` / `fff-grep`）许可证（MIT）
      并锁定版本（0.10.6），API spike 完成（FuzzySearchOptions / GrepSearchOptions 12 字段、
      取消语义、SharedFilePicker 守卫与借用约束——契约测试 + 集成测试固化）
- [x] Rust `search/` 封装：常驻索引（Ai 模式 + watcher，AppState 挂载）、文件名 fuzzy
      （分页 + 字节偏移高亮数据）、内容 grep（固定串/正则、smart case、上下文行、
      取消令牌接力、nextFileOffset 分页游标）；进度走 `search_status` 轮询（300ms 自动启停）
- [x] 文件名搜索页（找文件）：防抖 80ms 即时过滤、命中高亮（UTF-8 字节偏移→字符下标）、
      匹配分数、资源管理器定位（tauri-plugin-opener）
- [x] 内容搜索页（搜内容）：正则/固定串、智能大小写、上下文行数可调（0–10）、
      按文件分组 + 匹配计数、行号对齐、取消按钮 + 新搜索自动取消旧搜索、续页按钮
- [x] 共享配置：根目录历史持久化（`settings.search.rootHistory`，去重上限 8，datalist）
- [x] 性能验收（`大目录索引与首屏性能` #[ignore] 实测）：10 万文件索引 1.70s、
      fuzzy 首屏 85.8ms（命中 14500 条）——< 1s 验收线大幅达标；
      内容搜索引擎即 rg 内核（fff 同源）

设计变更：plain/regex 文件名模式热切换、整词、包含/排除 glob、预览窗格
降级为 0.1.x 打磨项（fff 不直接暴露这些旋钮，详见架构 §4.9 记录）。

## M3 · 截屏（v0.1，2026-09-03 完成核心）

**目标**：全局快捷键触发、六种标注工具、复制/保存/贴图，交互对齐微信截图。

- [x] `shots/` 模块：xcap 全屏捕获（多显示器枚举 + 各屏 PNG 冻结帧，
      物理坐标 + DPI 缩放换算；真实捕获链路 #[ignore] 测试 0.71s 通过）
- [x] 全局快捷键：`tauri-plugin-global-shortcut`，默认 Alt+A（随设置恢复注册）；
      快捷键管理 UI（设置页录制式输入、先试注册再落盘、冲突显式报错——
      实测微信占用 Alt+A 时显式 warn 不崩溃，换键即用）；
      设置持久化（`hotkeys.screenshot`，空串 = 停用）
- [x] 覆盖窗：每屏一窗全屏无边框置顶（`#/overlay` hash 路由），冻结画面、
      暗化遮罩、十字光标、拖框选区、8 手柄微调与整体移动、尺寸提示（W×H）、
      会话锁防重入（热键连击忽略）、窗口销毁自动解锁
- [x] 标注工具栏：矩形 / 椭圆 / 箭头 / 画笔 / 马赛克（区域像素化）/ 文字
      （点击定位直接键入，Enter 落墨），颜色（红黄蓝绿黑白）与三档线宽、
      撤销 / 重做（Shape 栈）、工具栏自动避让屏幕边缘
- [x] 产出：复制剪贴板（默认，双击选区同效）/ 保存截图目录
      （`图片\千寻截屏\qx-YYYYMMDD-HHmmss.png`）/ 贴图 Pin（置顶小窗、drag-region
      拖动、滚轮缩放、双击/Esc 关闭）/ Esc 全程取消
- [x] 托盘「截图」菜单（鼠标路，与热键同流水线）

打磨项（0.1+）：马赛克笔刷沿途涂抹（现为拖框区域式）、截图目录可配置、
跨屏拖选、多显示器真机全流程人工验证（单屏链路已验）。

## M4 · 多标签终端（v0.1，2026-09-03 完成核心）

**目标**：专业多标签终端，性能达标。

- [x] xterm 6 + webgl + Svelte 5 集成（onMount 装卸、fit + ResizeObserver 双向
      resize、OSC 标题跟随、webgl 失败自动降级 DOM 渲染）
- [x] `terminal/`（Rust portable-pty 0.9 全自研）：读泵（8KB 块→事件）+
      退出监视（先 emit 再清理——conpty drop(master) 可能阻塞）+ resize + kill
- [x] 多标签 UI：新建/切换/关闭（运行中确认）/标题随 OSC
- [x] 会话保活：标签切换 CSS 隐藏，xterm 实例与进程状态保留
- [x] 终端设置：shell（auto/pwsh/powershell/cmd）、字号（8–32）、回滚行数
      （100–100k），新建标签生效
- [x] conpty 实战坑固化（#[ignore] 真实回环测试 113ms）：DSR `ESC[6n` 必须应答、
      收尾死锁链（ClosePseudoConsole ↔ reader 管道）与规避顺序
- [x] DSH 版本策略（pinned/existing + 锁定版本输入）——M0/M1 已交付，此处覆盖
- [ ] 性能验收（真机人工）：cat 50MB 不卡 UI；粘贴 1MB 不丢字符；8 标签 < 400MB

## M5 · 笔记 MVP（v0.1，2026-09-03 完成核心）

- [x] 笔记库数据层：目录初始化（默认 文档\千寻笔记）、frontmatter 轻解析
      （title/tags，与桥插件逐字一致）、原子写、.trash 软删、越界路径防护
- [x] 编辑器：CodeMirror 6（markdown + 嵌入语言）+ marked 预览 + 手动保存
- [x] 笔记检索：列表内标题/标签/路径过滤（Rust 侧全量 walk，最近修改优先）
- [x] 设置：notes.vaultDir（首用初始化引导）
- [ ] 验收补课（真机人工）：1000 篇笔记流畅；断电不丢（原子写已实现，待实测）

## M6 · DSH Host 插件桥 + AI（v0.1，2026-09-03 完成核心）

- [x] `qx-bridge` 插件（零依赖纯 ESM，内嵌二进制裁剪部署；不走独立工程——
      include_str! 落盘 profile node_modules + cordis.patch.yml 根 insert）
- [x] agent 工具：note_search / note_read / note_write（+ 系统提示注入）
- [x] AI 整理：`POST /qx/notes/organize` → `llm.stream()`（text-delta 聚合）；
      笔记页指令面板 + 结果存为新笔记
- [x] 千寻端回环调用（fetch DSH origin + CORS；CSP connect-src 放行回环）
      ——共享 token 延后到移动端接入时（当前个人回环场景无需求）
- [x] 部署器：bridge_deploy（幂等）/ bridge_status（三处事实核对）/ 启动自愈
- [x] 验证：本机 DSH 同源注入实测——fiber 激活、工具进表、路由 204、
      真实 deepseek-chat 整理返回；千寻侧端到端待用户真机走查

## R1 · EasyTier 远程访问（v0.1，2026-09-03 完成核心）

- [x] 全自研网关 `remote/`（axum）：网卡列表选择绑定（EasyTier 识别置顶）、
      端口可配置（默认 17400）
- [x] 配对：设备记录（id + 256bit token）+ URL 二维码（设置页）+ 吊销
      （token 失效 + 网关重建即全断）
- [x] 安全模型：`/qx-gate` 唯一免鉴权入口发 HttpOnly cookie；其余路径
      cookie/query 鉴权；DSH 始终回环；`enabled=false` 零监听（任务不启动）
- [x] 流量面：HTTP/SSE 流式转发 + events.mux/host WS 帧级双向桥
- [x] 生命周期：设置更新 / DSH 就绪-停止事件幂等 sync
- [ ] 真机验收（需 EasyTier 实网 + 手机）：扫码配对 → 手机开 DSH 全功能
      界面；吊销即时断连；关闭开关后端口扫描无监听

## R2 · Capacitor Android（v0.1 骨架 → v0.2 出包）

> 工具链与构建环境 2026-09-03 就位（`D:\programs`，见 V0.2 P3 节）；
> mobile/README.md 记录完整构建路径。

- [x] Capacitor 7 壳工程骨架（`mobile/`）：config（androidScheme https）、
      构建脚本与说明
- [x] 配对入口页（www/）：粘贴配对链接（形态校验）→ localStorage 记住
      → 自动直达工作台；深色跟随（prefers-color-scheme）+ 安全区
      （viewport-fit=cover + safe-area-inset）
- [x] 明文 HTTP 放行脚本化（`pnpm add:android` 自动执行
      scripts/apply-android-config.mjs，幂等可重放）
- [x] `pnpm add:android` 生成工程（android/ 入库）并构建 debug APK
- [ ] 配对流程真机走通；移动体验调优（DSH Web 手机端布局按需微调）
- 设计变更记录：原「bridge 移动优先页面」取消——手机经网关直接用
  DSH 自适应 Web，壳只做入口（少维护一套 UI；如后续体验不足再议）

## S1 · 同步（v0.1，2026-09-03 第一阶段交付）

- [x] 同步范围落地：**只同步笔记库一个目录**（ADR-013）；
      截图/settings/profiles 白名单留第二阶段；`.credentials.yaml`、
      sessions 永不同步
- [x] 第一阶段：vault 走 git——`sync/` 域（git 存在性探测/初始化仓
      （含 .trash 忽略 + 首提交）/推送 = 自动提交 + push/拉取 = rebase
      autostash）；设置页同步卡（状态 + 推/拉按钮 + 输出回显）
- [ ] 第二阶段（按需）：hub 同步插件（EasyTier 网内收发）、
      截图目录与 settings 白名单

## V0.2 · 功能深化（2026-09-03，设计 docs/05，P0→P3 四阶段）

**目标**：把 v0.1 六个域从「骨架」推到「可用」——实测反馈逐条闭环。

### P0 可用性修复 ✅

- [x] 终端（§1）：首会话渲染门（id=0 被 `{#if tab.id>0}` 吞掉的根因）+
      输出回放（64KB 上限，消 banner 竞态）+ 失败原因上屏可重试 +
      kill 显式 ChildKiller + 增量 UTF-8 解码 + 去原生 confirm + cwd 透传
- [x] 笔记（§2.1）：CodeMirror 游离视图修复（$effect 跟随容器挂载）+
      notes_save 结构化 frontmatter（修复首存丢元数据）+ 自动保存 +
      列表摘要 + PromptDialog/ConfirmDialog 通用组件
- [x] 远程（§5.1）：sync() 配置指纹（enabled/bind/port/devices 哈希）——
      配对/吊销/换网卡换端口即时生效；axum 内存级集成测试
      （配对 302+cookie → 转发 200 → 吊销 401）

### P1 体验重构 ✅

- [x] 搜索 Rust（§3.3）：search_list_drives 盘符枚举、FileHit size/mtime、
      search_content 流式化（Channel 逐片推送 + 800ms 分片预算 + 锁不跨片 + 总量上限）、glob 过滤（`*`/`?` 不跨段、`**` 跨段）
- [x] 找文件页（§3.1）：Everything 式可排序表格（名称/大小/修改时间）+
      类型过滤 chips + Ctrl/Shift 多选 + ↑↓/Enter/Ctrl+Shift+C 键盘流 +
      右键菜单（打开/定位/三种路径形态/批量复制）+ 盘符胶囊行
- [x] 搜内容页（§3.2）：即输即搜流式渲染 + 整词（\b 包装）+ glob 输入 +
      停止按钮 + 实时扫描计数 + 命中/文件右键菜单；取消语义修复（async 化，
      主线程不再被 3s 预算阻塞）
- [x] 截屏（§4）：删「选择」按钮——统一优先级分发器（文字提交 > 选区手柄 >
      新增边带 ≤5dpr > 标注编辑 > 绘制 > 重开），框完即可拖，
      双击复制仅无工具态

### P2 远程闭环 ✅

- [x] 一级「远程」页（§5.2）：启用开关/网卡下拉（EasyTier ⚡ 置顶 + 空态
      引导）/端口/状态行/配对（二维码 + 复制链接）/设备列表（吊销确认）/
      自检按钮（remote_self_check：带真实 token 走 /qx-gate）
- [x] pair/revoke 保存后自触发 sync（不再依赖前端补发「应用」）
- [x] 真机验收（§5.3）：2026-09-04 真机过壳链路——PKB110 实机经网关
      配对（302+HttpOnly cookie）→ 转发 → 壳内完整 DSH 工作台；
      EasyTier 实网段用 `adb reverse` USB 隧道替代，配对/网关/转发/DSH
      全链路真实；吊销即时 401 由集成测试覆盖
- [x] 真机回归揪出并修复两处隐性缺陷：① sync() 指纹不含上游端口——
      网关先于 DSH 就绪启动（上游 0 占位），就绪事件再 sync 因指纹未变
      不重建，网关永远指向上游 0（「远程完全不可用」的实锤根因）；
      ② 并发 sync 抢绑端口 10048 且旧任务 abort 后未等监听器释放，
      重建即全灭——sync 全程互斥 + abort 后 await 旧任务退出

### P3 Android 出包 ✅

- [x] 工具链进 `D:\programs`（§6.1）：JDK 21（Temurin，JAVA_HOME）+
      cmdline-tools（ANDROID_HOME）+ platform-tools/android-35/build-tools 35
- [x] 壳工程（§6.2）：mobile 独立 lockfile（--ignore-workspace）、
      cap add android 入库、明文放行脚本化（scripts/apply-android-config.mjs
      幂等）、gradle 腾讯镜像 + 阿里云 maven 镜像
- [x] gradlew assembleDebug 出 debug APK
- [x] adb install 真机验收（§6.3）：PKB110（ColorOS）安装/启动/壳内
      配对直达工作台全通过；真机暴露并修复壳外跳浏览器问题
      （Capacitor 7 的 allowNavigation 在 server 节，HostMask 语义 '*'
      匹配任意主机——放 android 节无效）

## 里程碑间的纪律

1. 任何里程碑开工前，先读 `03-编码规范`，门禁脚本必须已在跑；
2. 里程碑中途发现设计问题：先改设计文档（02/04）再改代码，文档与实现不许长期漂移；
3. 每个里程碑结束打 tag 并写简短 release note。
