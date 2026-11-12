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
- [x] 应用标识与命名：`com.qianxun.desktop`、窗口标题"千寻"

**验收**：`pnpm tauri dev` 打开千寻窗口，导航/主题/托盘可用，门禁全绿。

## M1 · 核心托管（v0.1）✅ 已完成

**目标**：检测 → 安装 → 固定端口启动 → 守护 → 承载 DSH，全链路可用。

- [x] 移植 `proc-guard`、`node-runtime` 两个 crate（自有化，测试全保留）
- [x] 移植 supervisor 并**改造为固定端口**：`--port <配置>`、就绪行校验端口一致性、
      冲突显式报错（含保留端口 10000 专门提示）、健康探测、退避重启
- [x] **DSH_HOME 隔离**（ADR-009）：spawn 时注入 `DSH_HOME=<数据目录>/dsh-home`
- [x] DSH 安装：私有 prefix **pnpm 安装**（pnpm 每次安装取最新、失败回落已装版本，
      备份→直装→校验事务，registry 按 `mirrors.npmRegistry` 默认 npmmirror，
      allowBuilds 放行原生构建）
- [x] Node 安装：系统 curl.exe 下载 win-x64 zip + SHASUMS256 校验 +
      解压到 `node/`（auto=npmmirror 优先失败转官方），不动系统 PATH
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
      按文件分组 + 匹配计数、行号对齐、取消按钮 + 新搜索自动取消旧搜索
      **（续页按钮降级为 0.1.x 打磨项：流式循环已覆盖全集，当前不需要手动翻页；
      `nextFileOffset` 契约字段保留，前端未消费）**
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
      cookie/query 鉴权；DSH 始终回环；`enabled=false` 时**不绑 LAN**
      （网关仍启动并只听回环，DSH 页 iframe 依赖它）
- [x] 流量面：HTTP/SSE 流式转发 + `/api/remote.mux` WS 帧级双向桥
      （10 分钟空闲超时，防半开连接）
- [x] 生命周期：设置更新 / DSH 就绪-停止事件幂等 sync
- [x] 配对凭据：URL 只带**一次性 5 分钟 code**（docs/18 A4），长期 token
      只落在兑换后的 cookie；token 比较为常量时间
- [ ] 真机验收（需 EasyTier 实网 + 手机）：扫码配对 → 手机开 DSH 全功能
      界面；吊销即时断连；**关闭开关后 LAN 侧端口扫描无监听**
      （回环侧仍应监听——纠偏记录见 docs/18 B5，原文「端口扫描无监听」
      未区分两侧，与实现矛盾）

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

## V0.2.1 · DSH 0.1.2 适配 + 外壳整合（2026-09-05）

- [x] DSH 版本硬编码（ADR-015）：`install::PINNED_VERSION` 随千寻版本
      锁定（当时 0.1.2-rc.1），删用户可编辑的 dsh.pinnedVersion——
      软件版本 ↔ DSH 版本一一对应，改版本=升级千寻
- [x] DSH 0.1.2 浏览器鉴权适配：readiness 保留就绪 URL 完整 token
      （回环/http/显式端口校验不放宽）；Status::Ready 拆分 origin+token
      （凭据不进展示字段）；DSH iframe 用 token URL 兑换签名 cookie
- [x] 网关代持 DSH 鉴权：启动时（及上游 401 时）用 token 兑换
      dsh-auth-* 签名 cookie 并注入每次上游转发与 WS 桥；token 与
      DSH cookie 均不出网关进程；转发剥除手机侧 Host/Origin/Sec-Fetch
      （DSH 的 Host/Origin 栅栏按上游 authority 判定）；集成测试覆盖
      兑换→注入→剥除全链路
- [x] 外壳导航整合：「环境」+「控制台」合并为一级「环境」页
      （自上而下：检查 Node → 检查 DSH → 启动/停止 → 日志铺满剩余
      空间，安装/启动日志全程可见）；「DSH」保持为 DeepSeek Harness
      聊天页（iframe），启动入口收敛到环境页

## V0.4 · 千寻 0.4 适配锚点切换（DSH 0.1.5）+ 版本号语义落地

**目标**：把 V0.2.1 时随 `install::PINNED_VERSION` 锁住的 DSH 0.1.2 升级到
DSH 0.1.5，并确立千寻版本号 ↔ DSH 适配锚点的对应关系（ADR-015）。

### 版本号语义（ADR-015）

千寻版本号格式 `0.<DSH 适配锚点>.<千寻修订号>`：

- **主版本号第二位** = **DSH 适配锚点代号**：千寻 `0.4.x` 适配 DSH `0.1.5`；
  锚点切换（如 `0.4.x` → `0.5.x`）= 一次 DSH 升级 = 一次完整 DSH 真机验收；
  锚点代号是约定不是机械推导（`0.4` ↔ `0.1.5` 由团队手工映射）。
- **第三位** = **千寻自身修订号**：`0.4.0` 是该主版本下的第一个发布版，
  `0.4.1` 是第二个修订版（仅千寻自身迭代，DSH 不动）；
  修订号递增不需要重验 DSH。
- 编号规则：**千寻主版本号推进 = 锚点切换 + 至少一次 `PINNED_VERSION` 升级
  - 一次完整 DSH 真机验收**；千寻修订号推进 = 千寻自身小迭代，可只跑千寻
    自身的回归。

### 落地项

- [ ] `install::PINNED_VERSION` 由 `0.1.2-rc.1` 升到 `0.1.5-rc.1` ✅（代码侧）
- [ ] 千寻版本号 `0.3.0` → `0.4.0` ✅（包元数据：root / tauri / mobile / Cargo.lock / mobile lockfile / iOS MARKETING_VERSION / Android versionName）
- [ ] ADR-015 登记于 `02-技术架构.md §6` ✅
- [ ] `install::PINNED_VERSION` 注释引用更新为 ADR-015 ✅（V0.2.1 那行 ADR-013 引用同步修正）
- [ ] **版本一致性兜底**：`check_installed_at_version` 校验完整性 + 版本号 = `PINNED_VERSION`；`launch_plan` 在不匹配时拒绝启动（返回 `Error::DshVersionMismatch`）✅
- [ ] **UI 警告**：环境页 DSH 块在 `dshVersionMatches=false` 时显示 `⚠ 版本不匹配，需重新安装` + 「千寻要求 X，当前 Y」副说明；按钮文案变「升级」；启动按钮禁用 ✅
- [ ] install.rs 单元测试 `版本一致性校验拦截旧版DSH` / `版本一致性校验在目录缺失时返回false` ✅
- [ ] **插件市场**（架构 §4.11）：**双数据源 + 浏览在前端**——推荐源
      `dsh-plugin-catalog`（前端 tar.gz 解包，零新增 crate）+ 搜索源
      registry search（webview fetch 走系统代理，CSP 放行 https:）；
      Rust 只留变更面（安装预检 / 卸载 / 已装清单），弃用与不兼容拒绝安装，
      `@deepseek-ai/*` 内核不可卸载；插件页 = 推荐 / 搜索 / 已安装 三 tab ✅
- [ ] **环境页减法**：删 Node/DSH 常驻信息卡——健康时左侧为空、日志即主体；
      只在缺 Node / DSH 未装 / 版本不匹配时显示紧凑操作行；版本/路径/
      工作目录/DSH_HOME 收进默认收起的「信息」块 ✅
- [ ] **笔记桥迁移**：部署入口从插件页迁到笔记页状态条（三处事实就位即隐藏）✅
- [ ] DSH 0.1.5 真机验收（按 V0.2.1 纪律：readiness 行格式 / 鉴权契约 / iframe 链路 / 网关代持）
- [ ] 插件市场真机验收（推荐目录拉取 + 分类过滤 → 安装 → 重启 DSH 生效 → 卸载；搜索兜底）
- [ ] tag + 简短 release note（按 §「里程碑间的纪律」第 3 条）

## V0.5 · 千寻 0.5 适配锚点切换（DSH 0.1.6）

**目标**：DSH 适配锚点由 `0.1.5` 切到 `0.1.6`（V0.5.1 起当前钉 `0.1.6-alpha.2`），
千寻版本号 `0.4.0` → `0.5.0` → `0.5.1`。锚点代号映射：`0.5` ↔ `0.1.6`（ADR-015，手工约定）；
V0.5.1 仅同锚点内的 alpha 递增（`0.1.6-alpha.1` → `0.1.6-alpha.2`），属于修订号——
锚点未切换，按 ADR-015 不需要重新验证 DSH。

### 落地项

- [ ] `install::PINNED_VERSION` 由 `0.1.5-rc.1` 升到 `0.1.6-alpha.1` ✅（代码侧）
- [ ] `install::PINNED_VERSION` 由 `0.1.6-alpha.1` 升到 `0.1.6-alpha.2` ✅（V0.5.1，修订）
- [ ] 千寻版本号 `0.4.0` → `0.5.0` ✅（包元数据：root / tauri / mobile / Cargo.lock / mobile lockfile / iOS MARKETING_VERSION / Android versionName）
- [ ] 千寻版本号 `0.5.0` → `0.5.1` ✅（V0.5.1，修订；同上 6 处包元数据）
- [ ] install / market 单元测试通过（兼容判定对 rc/alpha 钉版「原样 + 去预发布」双形态宽松语义不变）✅
- [ ] DSH 0.1.6 真机验收（按 V0.2.1 纪律：readiness 行格式 / 鉴权契约 / iframe 链路 / 网关代持）
- [ ] 插件市场真机验收（推荐目录拉取 + 安装 → 重启 DSH 生效 → 卸载）
- [ ] tag + 简短 release note（按 §「里程碑间的纪律」第 3 条）

## V0.6 · 千寻 0.6 适配锚点切换（DSH 0.1.7）

**目标**：DSH 适配锚点由 `0.1.6` 切到 `0.1.7`（当前钉 `0.1.7-alpha.2`），
千寻版本号 `0.5.2` → `0.6.0`。锚点代号映射：`0.6` ↔ `0.1.7`（ADR-015，手工约定）；
`0.1.7-alpha.1` 已在 npmjs / npmmirror 双源确认存在。

### 落地项

- [ ] `install::PINNED_VERSION` 由 `0.1.6-alpha.2` 升到 `0.1.7-alpha.1` ✅（代码侧）
- [ ] 千寻版本号 `0.5.2` → `0.6.0` ✅（包元数据：root / tauri / mobile / Cargo.lock / mobile lockfile / iOS MARKETING_VERSION / Android versionName）
- [ ] `PINNED_VERSION` `0.1.7-alpha.1` → `0.1.7-alpha.2` ✅（0.6.1 修订，代码侧；配套：内置搜索 fetch provider 接入 moli（web_fetch 真实状态码/JS 渲染/降噪）+ 设置卡 register API 适配守卫）
- [ ] `PINNED_VERSION` `0.1.7-alpha.2` → `0.1.7-rc.1` ✅（0.6.2 修订，代码侧；配套：千寻版本号 0.6.1 → 0.6.2 全量包元数据同步）
- [ ] `PINNED_VERSION` `0.1.7-rc.1` → `0.1.7-rc.2` ✅（代码侧；配套：内置 playwright 工具改 launcher + `moli serve --layout` + `--cdp-endpoint`（moli 非 Chromium 壳，executablePath 直拉必败）+ 截图微信式窗口感知 `shots_window_at`）
- [ ] DSH `0.1.7-rc.2` 真机验收（应用内升级运行时后过一遍启动/搜索/抓取）
- [ ] iOS MARKETING_VERSION / Android versionName 漂移修正 ✅（0.5.1 / 0.5.2 两次修订漏同步原生壳，停在 0.5.0，本次一并归位）
- [ ] install / market 单元测试通过（兼容判定对 rc/alpha 钉版「原样 + 去预发布」双形态宽松语义不变）
- [ ] DSH 0.1.7 真机验收（按 V0.2.1 纪律：readiness 行格式 / 鉴权契约 / iframe 链路 / 网关代持）
- [ ] 插件市场真机验收（推荐目录拉取 + 安装 → 重启 DSH 生效 → 卸载）
- [ ] tag + 简短 release note（按 §「里程碑间的纪律」第 3 条）

## V0.7 · 千寻 0.7 适配锚点切换（DSH 0.2.0）

**目标**：DSH 适配锚点由 `0.1.7` 切到 `0.2.0`（当前钉 `0.2.0-rc.2`），
千寻版本号 `0.6.2` → `0.7.0`。锚点代号映射：`0.7` ↔ `0.2.0`（ADR-015，手工约定）。

### 落地项

- [ ] `install::PINNED_VERSION` 由 `0.1.7-rc.2` 升到 `0.2.0-rc.2` ✅（代码侧）
- [ ] 千寻版本号 `0.6.2` → `0.7.0` ✅（包元数据：root / tauri / mobile / Cargo.lock / mobile lockfile / iOS MARKETING_VERSION / Android versionName）
- [ ] install / market 单元测试通过（兼容判定对 rc 钉版「原样 + 去预发布」双形态宽松语义不变）
- [ ] DSH `0.2.0-rc.2` 真机验收（应用内升级运行时后过一遍启动/搜索/抓取）
- [ ] DSH 0.2.0 真机验收（按 V0.2.1 纪律：readiness 行格式 / 鉴权契约 / iframe 链路 / 网关代持）
- [ ] 插件市场真机验收（推荐目录拉取 + 安装 → 重启 DSH 生效 → 卸载）
- [ ] tag + 简短 release note（按 §「里程碑间的纪律」第 3 条）

## V0.8 · 千寻 0.8 适配锚点切换（DSH 0.2.1）

**目标**：DSH 适配锚点由 `0.2.0` 切到 `0.2.1`（当前钉 `0.2.1-alpha.1`，上游
tag `dsh-v0.2.1-alpha.1`），千寻版本号 `0.7.0` → `0.8.0`。锚点代号映射：
`0.8` ↔ `0.2.1`（ADR-015，手工约定）。

### 上游这一版的变更（2026-10-03 发布）

- 新增实验性 Claude Code Mods 兼容层、「让 Agent 创建插件」、可选 DevTools
  组合包、`--public-url`（带路径前缀的反代入口）
- 自动化任务改为 Web 内置；提醒工具按模式提供（极简模式/子代理不可用）
- **破坏性**：移除运行时 invariant 插件及各包 `./invariant` 导出
- **破坏性**：输入区统计扩展拆为 `activity` / `usage` 两个入口
- 子路径插件不再读独立 `package.json`，显示文本/图标走子路径导出
- 桌面端默认改系统分配端口（规避 Windows 保留端口范围）

### 依赖树实测（npm registry diff，2026-10-07）

- `@deepseek-ai/dsh-experimental-schedule-bundle` **在 0.2.1-alpha.1 未再发布**
  （npm 最后一版是 `0.2.0-rc.2`），调度改为 Web 内置的 `dsh-schedule` +
  `dsh-tool-schedule` + `dsh-client-ui-schedule`。**现存用户 profile 的 bundles
  列表里正挂着这个包**（`~/.qianxun/dsh-home/profiles/web/package.json`），升级后
  成为悬空条目——要么 DSH 自动清理，要么复现 modsearch 当年那条
  `cannot resolve profile bundle` 警告。上真机第一件事就是看这条。
- `@deepseek-ai/dsh-client-connection` 0.2.0-rc.2 → 0.2.1-alpha.1：千寻回环代理的
  鉴权契约面。**已探针实测未变**（见下）。
- `./invariant` 导出移除：全仓 grep 无引用（含 `plugins/decision-laya`），无影响。
- `@qianxun/decision-laya` 走 root 导出（`.`），不涉及「子路径插件不再读 package.json」。
- `@liustack/modsearch` 早已从 profile 卸载（settings 里的 `plugins.pinned` 只是钉版
  记录，不强制安装），`stats` → `activity`/`usage` 拆分打不到它。

### 隔离探针实测（`.tmp/dsh-0.2.1-probe`，2026-10-07）

用与 `install.rs` 完全相同的 pnpm 参数装 0.2.1-alpha.1，用 supervisor 的启动契约
（`--profile web --no-open --host 127.0.0.1 --port N` + `DSH_HOME`/`DSH_DESKTOP`/
`QIANXUN_VERSION`）拉起：

| 契约项 | 结果 |
| --- | --- |
| 安装 | ✅ 14.6s，`allowBuilds` 白名单命中（node-pty / koffi / protobufjs / subprocess-local 全跑完） |
| 就绪行 | ✅ 仍是 `dsh web: http://127.0.0.1:17999/?token=…`，`readiness::parse` 原样吃下 |
| cookie 兑换 | ✅ `GET {ready_url}` → `303` + `set-cookie: dsh-auth-…`，前缀常量仍有效 |
| 鉴权强制 | ✅ 带 cookie 过 401 落到路由（404），不带 cookie 直接 401 |
| 页面引导 | ✅ `__DSH_BOOT__` 正常，条目含新增的 `dsh-client-ui-schedule` |

结论：**千寻依赖的那几条外部契约全部未变**。余下风险只在 DOM 层
（mobile-access `custom.js` 注入）与千寻自身 UI，需要真机过一遍。

### 上线后发现的两处问题与修复

用户装完新版本后反馈了安装日志里的 `npm warn allow-scripts` 噪音，顺带把上一节
那两个「真发现」一并结清。两处都是同一类毛病：**功能在某条路径下是通的，但用户
既没有干净的反馈，也没有入口去收拾它**。

#### 1. npm 11.17 的 install-script 门禁噪音

用户贴的安装日志里有 4 行：

```
npm warn allow-scripts @pnpm/exe@11.7.0 has no allow-scripts config in the project
npm warn allow-scripts @pnpm/exe@11.7.0 has no allow-scripts config in the project
npm warn allow-scripts @pnpm/win-x64@11.7.0 has no allow-scripts config in the project
```

含义是 npm 11.17 新装了 install-script 白名单门禁：`@pnpm/exe` 有
`preinstall: node setup.js`（从 `@pnpm/win-x64` 平台包取二进制做 SEA 注入），而项目
清单里没有对应的 `allowScripts` 条目。**实测默认是「警告但照跑」**——pnpm.exe 照样
落地 98MB 且 `--version` 正常，所以这是噪音而非故障；但它出现在用户可见的安装日志里
就像坏了，而且 npm 还留着 `--strict-allow-scripts` 把警告变硬错，真到那天 pnpm 会
静默不落位。

npm 给的补救是让用户敲 `npm approve-scripts`——桌面上没人会敲。CLI flag 也不通：
`npm install --allow-scripts=@pnpm/exe` 在 project-scoped 安装下被 npm 直接拒
（*"not allowed in project-scoped installs"*），它要求写进清单或 `.npmrc`。

所以在 `install::ensure_pnpm_tool` 里、npm 安装之前先把条目合进
`<pnpm_tool_dir>/package.json` 的 `allowScripts`（`crate::atomic::write`）：键取
`PNPM_SPEC` 的包名部分（`@pnpm/exe`，不带版本，`PNPM_SPEC` 升版不用同步改键），
已就位就不碰文件，清单缺失或损坏就当空的重建，写失败只记 warn——顶多回到有 warn 的
现状，不该拦住一次本来能成功的安装。

顺带修正了 `PNPM_SPEC` 上那段已经不成立的注释（原写「exe 直接在 tarball 里——
没有 install 脚本」；11.7.0 恰恰带 preinstall）。

**真机验证**（debug 环境 `%USERPROFILE%\.qianxun_dev`，2026-10-08）：从
0.2.0-rc.2 升到 0.2.1-alpha.1，装完 allow-scripts 相关日志 **0 行**（升级前同一
条路径是 4 行），`<pnpm_tool_dir>/package.json` 落成
`{"allowScripts":{"@pnpm/exe":true},"dependencies":{"@pnpm/exe":"^11.7.0"}}`，
npm 装完保留了该字段，pnpm 走 pnpm 11.7.0 正常落位。

#### 2. 僵尸钉版没有清理入口

`settings.plugins.pinned` 里挂着 `@liustack/modsearch@5.10.3`，插件早已从 profile
里卸载（当初绕过 `market_remove` 从磁盘上摘的，所以没走 unpin）。问题在于它清不掉：

- 已装列表只按 `deployed_version` 列条目，这个插件不在其中 → 没有卸载按钮；
- 清单区只显示一个**数量**，不列条目；
- 而 `market_sync_pinned` 会照 `pinned` 记录把它按版本装回来。

原先只能停掉千寻、用脚本改 `settings.json` 顶上（`.tmp\unpin-modsearch.ps1`），
并且必须校验目标之外的字节没被动过——因为 `settings::update` 是整份从内存快照回写
（settings.rs:703），运行中改盘会被无声覆盖回去。

补了两件事：

- **`market::market_unpin`**：只做 `settings.plugins.unpin`，不碰 bundles 与
  node_modules（卸盘是 `market_remove` 的活）；同样受 busy 锁与安全模式冻结约束，
  成功后往 supervisor 记一行。
- **插件页「已安装」tab**：新增 `pinnedMissing` 派生量 = 清单里有、profile 里没装的
  条目，以警示块逐条列出并给「取消钉版」按钮（`data-testid="unpin"`），两个 tab 分支
  都渲染。

**真机验证**（同一个 debug 环境，2026-10-08）：在 dev settings 里种了同名僵尸钉版
当夹具 → 区块按预期出现（`@liustack/modsearch 5.10.3` + 按钮）→ 点「取消钉版」→
区块消失、dev `settings.json` 的 `pinned` 变空数组、日志留下
`[插件] 已取消钉版：@liustack/modsearch`。

### 落地项

- [ ] `install::PINNED_VERSION` 由 `0.2.0-rc.2` 升到 `0.2.1-alpha.1` ✅（代码侧）
- [ ] 千寻版本号 `0.7.0` → `0.8.0` ✅（包元数据：root / tauri / mobile / Cargo.lock / mobile lockfile / iOS MARKETING_VERSION / Android versionName）
- [ ] install / market 单元测试通过（兼容判定对 alpha 钉版「原样 + 去预发布」双形态宽松语义已覆盖：`0.2.1-alpha.1` 与 `0.2.1` 双试）
- [ ] Rust 全量 `cargo fmt` + `clippy -D warnings` + `test` ✅（431 passed / 0 failed / 6 ignored，含新增的两条白名单单测）
- [ ] 前端 `svelte-check` ✅（0 error）、`vitest` ✅（297 passed）、资产校验 ✅
- [ ] 前端 `prettier` / `eslint` ❌ **分支既有欠账，非本次引入**：`mobile-flutter/`
  下 `scripts/probe_dom_structure.js` 用 Python 式 `"""` 开头（prettier/eslint 都解析
  不了）、`qx-mobile/custom.js` 与 `.tmp/probe_*.js` 有 unused var，共 27 error。
  发版前要么把那批探针挪出 lint 范围，要么修成合法 JS。
- [x] **npm allow-scripts 门禁消噪** ✅（代码 + 真机都过了）：见上节第 1 条。
- [ ] DSH `0.2.1-alpha.1` 真机验收：启动 / 搜索 / 抓取 / 网关代持 / 插件市场安装→重启→卸载
- [ ] **mobile-access 回归**（最脆的一环：`custom.js` 靠注入 DOM 进 DSH 页面，
  DSH web UI 一改就碎）——`window.qxMobile.register` 仍被调用 + 真机窄屏排版
- [ ] pinned 插件 `@liustack/modsearch@5.10.3` 在 0.2.1 上验证（`stats` → `activity`/`usage` 拆分会打到它）
- [x] **僵尸钉版的清理入口** ✅（代码 + 真机都过了）：新增 `market::market_unpin`
      与插件页「取消钉版」按钮（见上节第 2 条）。生产 `.qianxun\settings.json` 里
      那条 `@liustack/modsearch@5.10.3` **仍未清**——它只能由用户在跑着的千寻里点
      按钮（`settings::update` 会把运行期间的外部改动整份覆盖回去），等这版发出去
      之后在插件页点一下即可。备用的 `.tmp\unpin-modsearch.ps1` 脚本作废。
- [ ] 桌面端端口策略交叉验证：千寻固定 `dsh.port`（17300）与上游「系统分配端口」不冲突
- [ ] tag + 简短 release note（按 §「里程碑间的纪律」第 3 条）

## V0.8.1 · DSH 0.2.1-alpha.1 → 0.2.1-alpha.2（同锚点修订，2026-10-09）

**目标**：锚点仍是 DSH `0.2.1`，只把钉版从 `0.2.1-alpha.1` 递到 `0.2.1-alpha.2`
（上游 2026-10-09 发布，npm dist-tag `alpha`），千寻版本号 `0.8.0` → `0.8.1`。
按 ADR-015，这属于**千寻修订号**而非锚点切换：DSH 契约面没动，跑千寻自身回归即可。

### 依赖树实测（npm registry diff，2026-10-09）

顶层依赖的两类变化：

- **新增 7 个 experimental 包**：`dsh-experimental-badge-skill-bundle` /
  `-cot-translation-bundle` / `-ralph-bundle` / `-session-search` /
  `-session-titles-bundle` / `-terminal-bundle` / `-tool-worktree`。
  ralph 从工具（`dsh-tool-ralph`）改成了 bundle，是重定位不是删除。
- **移除 5 个包**：`dsh-webhook` / `dsh-webhook-github`（webhook 能力下线）、
  `dsh-hooks-claude-code` / `dsh-hooks-codex`（Claude Code Mods 兼容层下线，
  与 V0.8 记的「新增 Mods 兼容层」正好相反）、`dsh-tool-ralph`（并入 ralph bundle）。
  **仓内 grep 这 5 个包名：0 处引用**（含 `plugins/decision-laya`），无影响。
- 其余：全部 `@deepseek-ai/*` 子包 `0.2.1-alpha.1` → `0.2.1-alpha.2`；
  `cordis` / `schemastery` 等上游基座版本未动；`js-yaml ^4.2.0 → ^4.3.2`、
  `node-addon-require-builtin ^0.1.6 → ^0.1.9`。

### 契约面核验

| 契约项 | 结果 |
| --- | --- |
| `dsh-client-connection`（回环代理鉴权） | ✅ exports 未变 |
| `dsh-home-paths` / `dsh-app-boot` / `dsh-web-app` | ✅ exports 未变 |
| 顶层 `exports` / `bin` | ✅ 逐键 diff 无差异 |

结论：**千寻依赖的外部契约全部未变**。余下风险仍在 DOM 层（mobile-access
`custom.js` 注入）与千寻自身 UI，需要真机过一遍。

### 落地项

- [x] `install::PINNED_VERSION` 由 `0.2.1-alpha.1` 升到 `0.2.1-alpha.2` ✅（代码侧）
- [x] 千寻版本号 `0.8.0` → `0.8.1` ✅（包元数据：root / tauri / mobile / Cargo.lock / mobile lockfile / iOS MARKETING_VERSION / Android versionName）
- [x] 双源可解析 ✅（npmmirror 与 npmjs 都已同步 `0.2.1-alpha.2`；默认走 npmmirror，镜像滞后会让安装直接失败，必须提前确认）
- [x] Rust `cargo fmt --check` + `clippy --all-targets -D warnings` + `test --workspace`
      ✅（478 passed / 0 failed / 6 ignored）
- [x] 前端 `svelte-check` ✅（0 error / 13 warning，均为既有 a11y 提示）、
      `vitest` ✅（297 passed / 1 skipped）、`verify-assets` ✅；
      `prettier` / `eslint` ❌ **仍是 `mobile-flutter/` 下的分支既有欠账**
      （`scripts/probe_dom_structure.js` 用 Python 式 `"""` 开头，解析不了；
      `scripts/_dbg_harness.cjs` 整包按 CommonJS 写，`no-undef` 报了几百条），
      与本次改动无关，本次未新增一条
- [x] 兼容判定语义不变：`^0.2.x` 与 `^0.2.0-rc.1` 宽范围仍放行，`^0.1.x` 仍拦
      （`registry::兼容判定对rc钉版宽松` 用例硬编码范围，不随钉版漂移）
- [ ] DSH `0.2.1-alpha.2` 真机验收：启动 / 搜索 / 抓取 / 网关代持 / 插件市场安装→重启→卸载
- [ ] **mobile-access 回归**（同 V0.8 记录：`custom.js` 靠 DOM 注入，DSH web UI 一改就碎）
- [ ] 老 profile 里悬空的 `@deepseek-ai/dsh-experimental-schedule-bundle`
      条目（V0.8 首次发现）仍待用户点一次「取消钉版」；本次确认它在 alpha.2
      顶层依然不在，新版不提供自动清理
- [ ] `remote::online::tests::端到端_接入边缘并转发请求` 在**整个 workspace 并行跑**时
      偶发失败（mock 上游 HTTP server 抢不到端口，502），单跑 3/3 绿。与本次无关，
      但它是条真 flaky，值得单独排一次

## V0.8.2 · DSH 0.2.1-alpha.2 → 0.2.1-rc.3（同锚点修订，2026-10-24）

**目标**：锚点仍是 DSH `0.2.1`，钉版从 `0.2.1-alpha.2` 递到 `0.2.1-rc.3`（alpha
全部出清，基座转正），千寻版本号 `0.8.1` → `0.8.2`。按 ADR-015 属**千寻修订号**：
同锚点 DSH 升级，跑千寻自身回归 + 依赖树 diff，不做全套 DSH 真机重验。

### 依赖树实测（npm registry diff，2026-10-24）

顶层依赖 85 → 86：

- **移除 0 个**；
- **新增 1 个**：`@deepseek-ai/dsh-experimental-plugin-market-bundle`（插件市场
  experimental bundle，纯增量，market 兼容判定按钉版走不受影响）；
- 其余全部 `@deepseek-ai/*` 子包 `0.2.1-alpha.2` → `0.2.1-rc.3`；上游基座出 alpha：
  `cordis ~4.0.5-alpha.1 → ~4.0.5`、`schemastery ~3.18.5-alpha.1 → ~3.18.5`、
  `cordis-plugin-{timer,loader,include}` alpha → 正式。

### 落地项

- [x] `install::PINNED_VERSION` 由 `0.2.1-alpha.2` 升到 `0.2.1-rc.3` ✅（代码侧）
- [x] 千寻版本号 `0.8.1` → `0.8.2` ✅（包元数据：root / tauri / Cargo.lock 随构建；
      **mobile 系元数据未动**——mobile/ 与 mobile-flutter/ 有在制改动，发版时同步）
- [x] 双源可解析 ✅（npmmirror 与 npmjs 均已同步 `0.2.1-rc.3`，2026-10-24 实测）
- [x] Rust `fmt` + `clippy --all-targets -D warnings` + `test --workspace` ✅
      （2026-10-24 全绿；跳过两条既有失败：`intel::fetch::rss解析完整条目并过滤过期`
      ——intel 为在制模块；`remote::online::端到端_接入边缘并转发请求`——上节已记的真 flaky）
- [x] NSIS 发布包 `qianxun_0.8.2_x64-setup.exe` 构建成功 ✅（2026-10-24，11.6 MB）
- [ ] DSH `0.2.1-rc.3` 真机验收：启动 / 搜索 / 抓取 / 网关代持 / 插件市场安装→重启→卸载
- [ ] **mobile-access 回归**（`custom.js` 靠 DOM 注入，DSH web UI 一改就碎）

## V0.9.0 · DSH 0.2.1-rc.3 -> 0.2.2-alpha.2（锚点切换，2026-10-29）

**目标**：锚点从 DSH `0.2.1` 切到 `0.2.2`，千寻版本号 `0.8.2` -> `0.9.0`。按
ADR-015 属**锚点切换**：改钉版 + 跑千寻自身回归；全套 DSH 真机验收另行安排，
勾完才算锚点切换闭环。

### 依赖树实测（npm registry diff，2026-10-29）

顶层依赖 86 -> 91：

- **移除 0 个**；
- **新增 5 个**：`dsh-cua-bundle`、`dsh-experimental-forge-bundle`、
  `dsh-experimental-liveshare`、`dsh-official-bundles`、
  `dsh-personalized-recommendations`；
- 其余全部 `@deepseek-ai/*` 子包 `0.2.1-rc.3` -> `0.2.2-alpha.2`；上游基座出
  alpha：`cordis ~4.0.5` -> `~4.0.6-alpha.1`、`schemastery ~3.18.5` ->
  `~3.18.6-alpha.1`、`cordis-plugin-{include,loader,timer}` minor alpha。

### 落地项

- [x] `install::PINNED_VERSION` 由 `0.2.1-rc.3` 升到 `0.2.2-alpha.2` ✅（代码侧）
- [x] 千寻版本号 `0.8.2` -> `0.9.0` ✅（包元数据：root / tauri / Cargo.toml；
      Cargo.lock 随构建；mobile 系元数据未动，发版时同步）
- [x] 双源可解析 ✅（npmmirror 与 npmjs 均已同步 `0.2.2-alpha.2`，2026-10-29 实测）
- [x] 独立 DSH web 运行时已先行升级并冷启动自检 ✅（`~/.qianxun/dsh-runtime`
      2026-10-29 手动 `pnpm install` 升级，`bin.js --version` 自报
      `0.2.2-alpha.2`，koffi/node-pty 原生构建全过；旧版配置备份在
      `.tmp/backups/dsh-runtime-0.2.1-rc.3`）
- [x] Rust `fmt` + `clippy --all-targets -D warnings` + `test --workspace` ✅
      （2026-10-29：fmt/clippy 绿；test 186 绿 + 1 既有失败
      `intel::fetch::rss解析完整条目并过滤过期`——V0.8.2 已记录的在制模块用例，
      fixture 固定 2024 年日期随真实时钟漂移，与本次无关；上节另一条 flaky
      `remote::online::端到端_接入边缘并转发请求` 本轮通过）
- [ ] DSH `0.2.2-alpha.2` 真机验收：启动 / 搜索 / 抓取 / 网关代持 / 插件市场安装->重启->卸载
- [ ] **mobile-access 回归**（`custom.js` 靠 DOM 注入，DSH web UI 一改就碎）

注意：上游基座本轮带 alpha（cordis / schemastery 均为 `*-alpha.1`），与 V0.8.2
「alpha 出清」方向相反——`0.2.2` 转 rc 时需复查这几个基座是否转正。

## V0.9.1 · DSH 0.2.2-alpha.2 -> 0.2.2-alpha.3（同锚点修订，2026-11-03）

**目标**：锚点仍是 DSH `0.2.2`，钉版从 `0.2.2-alpha.2` 递到 `0.2.2-alpha.3`，
千寻版本号 `0.9.0` -> `0.9.1`。按 ADR-015 属**千寻修订号**：同锚点 DSH 升级，
跑千寻自身回归 + 依赖树 diff，不做全套 DSH 真机重验。

### 依赖树实测（npm registry diff，2026-11-03）

顶层依赖 91 -> 90：

- **移除 1 个**：`dsh-cua-bundle`（CUA 能力包被上游撤下，千寻未引用）；
- **新增 0 个**；
- 82 个 `@deepseek-ai/dsh-*` 子包 `0.2.2-alpha.2` -> `0.2.2-alpha.3`；
- 上游基座 alpha 递进：`cordis ~4.0.6-alpha.1` -> `~4.0.6-alpha.2`、
  `schemastery ~3.18.6-alpha.1` -> `~3.18.6-alpha.2`、
  `cordis-plugin-{include,loader,timer}` 同步 alpha.1 -> alpha.2。

### 落地项

- [x] `install::PINNED_VERSION` 由 `0.2.2-alpha.2` 升到 `0.2.2-alpha.3` ✅（代码侧）
- [x] 千寻版本号 `0.9.0` -> `0.9.1` ✅（包元数据：root / tauri / Cargo.toml；
      Cargo.lock 随构建；mobile 系元数据未动，发版时同步）
- [x] 双源可解析 ✅（npmjs 与 npmmirror 均已同步 `0.2.2-alpha.3`，2026-11-03 实测）
- [x] 桌面端无已装 runtime 冲突 ✅（各 data dir 下均未安装 dsh-runtime，
      不存在 `DshVersionMismatch` 门禁拒启风险）
- [x] Rust `fmt --check` + `clippy --all-targets -D warnings` + `test --workspace` ✅
      （2026-11-03 三项 rc=0 全绿；fmt 用 --check 非侵入模式，避免连带重排
      工作区在制的 remote/* 代码；上一版记录的 `intel::fetch::rss解析完整条目
      并过滤过期` 本轮亦转绿——在制的 intel/fetch.rs 已把写死日历日期的 fixture
      改为按当前时钟动态生成，修复归属在制工作，未计入本次提交）
- [ ] DSH `0.2.2-alpha.3` 真机验收：启动 / 搜索 / 抓取 / 网关代持 / 插件市场安装->重启->卸载
- [ ] **mobile-access 回归**（`custom.js` 靠 DOM 注入，DSH web UI 一改就碎）

## V0.9.2 · DSH 0.2.2-alpha.3 -> 0.2.2-rc.1（同锚点修订，2026-11-05）

**目标**：锚点仍是 DSH `0.2.2`，钉版从 `0.2.2-alpha.3` 递到 `0.2.2-rc.1`，千寻
版本号 `0.9.1` -> `0.9.2`。按 ADR-015 属**千寻修订号**：同锚点 DSH 升级，跑
千寻自身回归 + 依赖树 diff，不做全套 DSH 真机重验。

### 依赖树实测（npm registry diff，2026-11-05）

顶层依赖 90 -> 90：

- **移除 0 个、新增 0 个**；
- 82 个 `@deepseek-ai/dsh-*` 子包 `0.2.2-alpha.3` -> `0.2.2-rc.1`；
- **上游基座全部转正**（兑现 V0.9.1 段落的复查项）：`cordis ~4.0.6-alpha.2` ->
  `~4.1.0`、`schemastery ~3.18.6-alpha.2` -> `~3.19.0`、
  `cordis-plugin-include ~1.0.11-alpha.2` -> `~1.1.0`、
  `cordis-plugin-loader ~1.0.7-alpha.2` -> `~1.1.0`、
  `cordis-plugin-timer ~1.1.8-alpha.2` -> `~1.2.0`。

### 落地项

- [x] `install::PINNED_VERSION` 由 `0.2.2-alpha.3` 升到 `0.2.2-rc.1` ✅（代码侧）
- [x] 千寻版本号 `0.9.1` -> `0.9.2` ✅（包元数据：root / tauri / Cargo.toml；
      Cargo.lock 随构建；mobile 系元数据未动，发版时同步）
- [x] 双源可解析 ✅（npmjs 与 npmmirror 均已同步 `0.2.2-rc.1`，2026-11-05 实测）
- [x] Rust `fmt --check` + `clippy --all-targets -D warnings` + `test --workspace` ✅
      （2026-11-05 三项 rc=0 全绿；fmt 首轮抓到两处我方反代修复引入的格式
       问题——gateway.rs 分支花括号、proxy.rs 超宽行——已修，未触碰在制代码）
- [ ] DSH `0.2.2-rc.1` 真机验收：启动 / 搜索 / 抓取 / 网关代持 / 插件市场安装->重启->卸载
- [ ] **mobile-access 回归**（`custom.js` 靠 DOM 注入，DSH web UI 一改就碎）

注意：基座从 alpha 跳到 minor 转正（cordis 4.0.6 -> 4.1.0 等），不是单纯
rc 递版——千寻侧与 cordis 插件协议相关的面（插件加载器、定时器）在真机验收
时多看一眼。

## 挂账 · DSH 0.2.2-rc.2 上架等待 → 已上架，钉版已切换（2026-11-12）

**结论**：⛔ 已撤回（2026-11-12）——原结论「在 GitHub 已发布、但在 npm 未上架，
本轮不动钉版」失效。2026-11-12 实测：`0.2.2-rc.2` 已在 npm 双源上架
（`npm view @deepseek-ai/dsh@0.2.2-rc.2 version` 在 npmjs 与 npmmirror 均返回
版本号；dist-tags `next: 0.2.2-rc.2`，`latest` 仍为 `0.2.2-rc.1`）。
按用户要求（2026-11-12），`install::PINNED_VERSION` 已切换为 `0.2.2-rc.2`；
升级流程其余项见下方执行清单（多数未闭环）。

### 实测依据（2026-11-11 历史快照，上架状态已被 2026-11-12 复核取代）

- GitHub release `dsh-v0.2.2-rc.2` 确实存在（commit `ddac967`，imccyu 发布
  2026-11-11 11:33，Pre-release）；tag 上本体包在 `apps/cli/package.json`
  （name `@deepseek-ai/dsh`，version `0.2.2-rc.2`）。
- npm 双源未上架：`registry.npmjs.org` 与 `registry.npmmirror.com` 的
  `@deepseek-ai/dsh` 均 404 该版本，dist-tags 仍为
  `{latest: 0.2.2-rc.1, alpha: 0.2.2-alpha.3, next: 0.2.2-rc.1}`；
  `0.2.2` 系只有 alpha.2 / alpha.3 / rc.1。
- **GitHub 源不能当安装通道**（用户问过「可否用 github 的版本」，实测否决）：
  - rc.2 tag tarball（codeload，40.7 MB / 23924 条目）内 `apps/cli/lib/*.js`
    数量为 **0**——只有 `src/`，**没有构建产物**；npm tarball 才是已构建产物。
  - `apps/cli` 的 144 个依赖里 **135 个是 `workspace:` 协议**
    （如 `@deepseek-ai/cordis workspace:~`），从 tarball 安装等于要求用户机跑
    完整 monorepo 的 pnpm workspace 构建 + 原生编译。
  - rc.2 release 无任何预构建资产（`expanded_assets` 为空）。
  - npm 才是首发渠道：rc.1 的 npm 发布时间 2026-11-04T09:20:32Z **早于**其
    GitHub release 的 2026-11-04T13:07:14Z 约 4 小时。
- 桌面端独立 runtime 已对齐 `0.2.2-rc.1`（`~/.qianxun/dsh-runtime`，冷启动自检
  通过），与当前钉版一致，不存在 `DshVersionMismatch` 拒启风险。
  （⛔ 已撤回（2026-11-12）：23:42 实测 `~/.qianxun/dsh-runtime` 为
  `0.2.2-alpha.3`，与安装版二进制自身钉版一致——该 exe 构建于 2026-11-04，
  早于 rc.1/rc.2 钉版改动，源码改钉版不影响已安装 release。dev 版数据根
  隔离在 `~/.qianxun_dev`，见 `src-tauri/src/paths.rs:33-37`。）

### 决定（用户 2026-11-11）

1. **放弃 GitHub 源**，继续走 npm，等新版本在 npm 上架；
2. **不预置「registry 预检 + 空窗期回退」兜底**——保持 ADR-015 的严格钉版
   语义（千寻不装未经自己验证过的 DSH 版本）；
3. **不做后台轮询**，由用户在上架后通知再执行升级流程。

### 正式版上架后的执行清单（沿用三轮升级的既定流程）

- [x] 双源可解析性复核 ✅（2026-11-12：npmjs 与 npmmirror 的
      `@deepseek-ai/dsh@0.2.2-rc.2 version` 均返回 `0.2.2-rc.2`；dist-tags
      `next: 0.2.2-rc.2`、`latest: 0.2.2-rc.1`、`alpha: 0.2.2-alpha.3`）
- [ ] 依赖树 diff（`npm view @deepseek-ai/dsh@<新版本> dependencies --json`；
      **404 时该命令返回的是错误对象，不是依赖表**——见到「移除 90 个包」这类
      假 diff 必须先确认包本身可解析）
- [x] 改版本落点之 `src-tauri/src/harness/install.rs` 的 `PINNED_VERSION`
      `0.2.2-rc.1` -> `0.2.2-rc.2` ✅（2026-11-12，用户指示；`install_spec()`
      及全部引用点随常量自动生效，无需散改）
- [ ] 其余 3 个版本落点（千寻自身版本号，随发版递增时同步）：
      root `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml`
- [ ] 千寻版本号递增（`0.9.2` -> `0.9.3`；若届时锚点仍为 DSH `0.2.2` 则属
      ADR-015 的千寻修订号，只跑千寻自身回归；若上游改锚点则按锚点切换做全套）
- [ ] 门禁：`cargo fmt --check` -> `cargo clippy --all-targets -- -D warnings`
      -> `cargo test --workspace`
- [ ] `pnpm tauri build` 出安装包，用户自行安装验证
- [ ] **websearch 回归**（rc.2 变更点，见下）
- [ ] DSH 真机验收：启动 / 搜索 / 抓取 / 网关代持 / 插件市场安装->重启->卸载
- [ ] **mobile-access 回归**（`custom.js` 靠 DOM 注入，DSH web UI 一改就碎）

### rc.2 变更对千寻的影响面（提前勘察，真机验收时重点看）

- **网页搜索默认提供方改用 DeepSeek Search API**（「旧提供方和自定义配置需按
  升级指南调整」）：千寻在 `src-tauri/src/websearch/commands.rs:212-216` 往 DSH
  的 `web` 插件 config 写 `searchProvider: qx-websearch` + `fetchProvider:
  qx-websearch`（PATCH_MARK / FETCH_MARK 见 `commands.rs:23-26`），插件侧在
  `packages/websearch/dsh/index.js:134-198` 通过
  `ctx.web.registerSearchProvider` / `ctx.web.registerFetchProvider` 注册提供方。
  查 rc.2 源码 `docs/subsystems/web.zh.md:130,179,188`：**四个接口与配置键
  全部仍在**，且提供方选择规则是「显式 id 优先，否则唯一可用者自动选，多个未
  配置则抛 `WEB_PROVIDER_AMBIGUOUS`」——千寻显式写 id 的做法仍然成立，但要真机
  回归确认。
  （注：`docs/22-DSH运行时工具拦截能力探查报告.md` 相关的 rc.2 源码抽取在
  `.tmp/rc2-src/subset`，该 subset 未包含 web 插件源码目录，故此结论来自
  rc.2 tag 的 `docs/subsystems/web.zh.md` 全文，需要时重新抽源码复核。）
- AgentTeam 改用统一具名子代理、中断工具默认关闭；工具失败给出更明确的错误
  类型与代码；Session 支持批量追加事件——影响自定义插件/SDK 集成，千寻目前无
  相关自定义，暂无影响面。

## 里程碑间的纪律

1. 任何里程碑开工前，先读 `03-编码规范`，门禁脚本必须已在跑；
2. 里程碑中途发现设计问题：先改设计文档（02/04）再改代码，文档与实现不许长期漂移；
3. 每个里程碑结束打 tag 并写简短 release note。
