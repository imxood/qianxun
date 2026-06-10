# 2026-06-09 启动体验 + Splash 窗口收尾

> 状态: 已完成 | 适用范围: qianxun-desktop | 最后更新: 2026-06-09

## 1. 背景

**用户反馈** (2026-06-09 下午): "tauri 程序启动后, 至少 15 秒时间屏幕是空白. 然后 点击 `新建任务` 没有反应."

**根因链** (本次会话追了 4 个根因):
1. `chatStore.init()` 在生产代码从未被调, Tauri `session_event` listener 从未注册 (P0 致命)
2. `sessionStore.create()` 客户端造 ID, 不调后端, send_message 必 404
3. 启动后无 splash, 用户看到 10s 白屏以为是卡死
4. Tauri 2.x 默认权限收口, `event.listen` 抛 "core:event:allow-listen" 错误, onMount await 死锁

**逐个修复 + 扩展, 形成了完整的"启动体验打磨"工作流**。

## 2. 本次会话完成的事 (按时间顺序)

### 2.1 P0 致命 bug: `chatStore.init` 未调用
- **症状**: Tauri `session_event` listener 未注册, send_message 后 SseEvent 被 Tauri 丢弃, 用户"发消息没响应"
- **修复**: `+page.svelte:onMount` 调 `await chatStore.init()`
- **关联 commit**: 之前

### 2.2 Lazy session create + 真项目列表
- **设计**: session_id 后端生成, 创建 session 推迟到首次发消息
- **前端**: `NewTaskButton` 只切 `uiStore.switchToNew()`, `chatStore.send(null, text)` 检测 null 时先 `await sessionStore.create()` 拿真 ID
- **项目列表**: `projectStore.loadAll` 调 `listSessions('all')` 按 `project_root` 去重 derive

### 2.3 Tauri event.listen 权限错误
- **症状**: `+page.svelte:47 Uncaught (in promise) event.listen not allowed. Permissions associated with this command: core:event:allow-listen, core:event:default`
- **修复**: 创建 `qianxun-desktop/src-tauri/capabilities/main.json` + `tauri.conf.json:app.security.capabilities` 引用
- **Tauri 2.x 关键**: 必须显式声明 capability, 默认无权限

### 2.4 启动 Splash 屏 (前端)
- **新建组件**: `qianxun-desktop/src/lib/components/layout/LoadingSplash.svelte`
  - 4 步进度 (starting/connecting/loading/ready) + 进度条 + 步骤详情
  - 顶部 banner 错误展示 + 重试按钮
  - 淡入动画 + 当前步骤 spinner
- **`+page.svelte`**: 加 `bootstrapped` / `bootStep` / `bootProgress` 状态, 顶层条件渲染 splash 或主布局
- **错误容错**: 任何 init 失败仍进主布局 + 顶部 banner 持续显示

### 2.5 后端 lazy init
- `state.rs::build()` 改用 `new_for_test()` 骨架 (<100ms)
- 后台 spawn 真 init: `tauri::async_runtime::spawn` + `spawn_blocking` + `futures::executor::block_on`
- `ensure_restored()` 幂等方法, 只在 `send_message` 入口调
- **关键陷阱**: `list_sessions_impl` **不能** 调 `ensure_restored()`, 否则 `cancel_session` 设的 `paused=true` 会被 `restore_from_disk` 覆盖

### 2.6 Timing 日志体系
- **Tauri 端**: 4 跳 timing (T0 setup / T1 build / T2 connected / T3 init done), 用了 `since_t0_ms` + `since_prev_ms` 双标注避免用户混淆累计/相对耗时
- **前端**: `[boot F0.0]` 等 console.log, 配合 Rust 端精确定位哪一跳慢
- **用户用日志证实**: Tauri 端总耗时 < 1s, **10s 空白是 Vite dev 模式冷编译**

### 2.7 Rust 日志本地化
- **问题**: tracing-subscriber 默认输出 ISO 8601 UTC (e.g. "2026-06-09T11:03:13.245617Z"), 不易读
- **方案**: 自定义 `FormatTime` impl, 用 `time` crate 输出 "2026-06-09 19:03:13.245" 本地时间
- **路径**: `qianxun-desktop/src-tauri/src/time.rs` (40 行, FormatTime impl)
- **tracing-subscriber 0.3.23 没有内置 LocalTime**, 自己实现

### 2.8 Tauri 官方 splash 方案
- **最终方案**: 第二个 splash window 走 `static/splashscreen.html` 静态, main window 走 Vite dev
- **4 个文件改动**:
  - `tauri.conf.json`: 加 `splashscreen` 窗口 (400x400, 无装饰, 置顶)
  - `static/splashscreen.html`: 纯静态 splash (logo + 千寻 + 进度条), 不走 Vite
  - `lib.rs`: `SetupState` Mutex + `set_complete` command + 50ms 后标 backend ready
  - `+page.svelte`: ready 后 invoke `set_complete({ task: 'frontend' })`
- **效果**: dev 模式启动立即看到琥珀色 splash 窗口, Vite 编译完无缝切主窗口

## 3. 关键技术决策

| 决策 | 理由 |
|---|---|
| **session_id 后端生成** | 客户端/后端 ID 命名空间必须统一, 否则 send 必 404 |
| **Lazy session create** | 用户新建任务不立即 invoke, 首次发消息才创建, 减少无效 session 数量 |
| **真项目列表 (listSessions derive)** | mock 数据违反事实源原则, 列表必须从持久化 derive |
| **LoadingSplash 在 +page.svelte 顶层** | 启动期间不渲染主布局, 用户看到 splash 而不是"还没有会话"静态空态 |
| **后端 `new_for_test()` 骨架 + spawn_blocking 真 init** | build() 同步 < 100ms, webview 启动不被 SQLite 同步 IO 阻塞 |
| **`ensure_restored()` 只在 send_message 入口调** | 避免 list_sessions 调它会覆盖 cancel_session 的 paused=true (回归测试已加) |
| **Tauri capability 文件** | Tauri 2.x 默认权限收口, 必须显式声明, 不写 = 全部 invoke 失败 |
| **Tauri timing 日志 since_t0 + since_prev 双标注** | 防止用户混淆"elapsed_ms 累计"和"阶段耗时" |
| **自定义 FormatTime (time crate)** | tracing-subscriber 0.3.23 没有 LocalTime, 升级到 0.3.27+ 风险大 |
| **官方 splash 方案 (双窗口)** | 走 `static/splashscreen.html` 不走 Vite 编译, splash 启动 < 50ms |

## 4. 数据驱动的诊断 (用户日志)

**完整时序** (用户 19:17:03 启动日志):
| 阶段 | 耗时 | 结论 |
|---|---|---|
| Tauri 启动 → T0 setup entered | 366ms | ✅ 优秀 |
| 后台 async init (provider + restore 16 sessions) | 13ms | ✅ 优秀 |
| Tauri setup 全部完成 (含 500ms 故意延迟) | 911ms | ✅ 优秀 |
| **Vite dev 模式冷编译 (从 19:17:03.8 到 19:17:11)** | **~7s** | ❌ **dev 模式固有限制** |
| Vite 首屏编译完 (ThreeColumnLayout a11y 警告) | 11 (3s 后) | Vite 按需编译 |

**关键数据**: Tauri 端总和 < 1s, 10s 空白 99% 是 Vite dev server 冷编译。

## 5. 修复后的效果 (用户验收)

| 场景 | 启动时间 | 体验 |
|---|---|---|
| `pnpm tauri dev` (dev 模式) | < 1s 看到 splash, 7-10s 后主窗口 | ✅ 启动有反馈, 不再黑屏 |
| `pnpm tauri build --no-bundle` (release 模式) | < 1s | ✅ 跳过 Vite, webview 直接读 build/ |

## 6. 教训

1. **Tauri 2.x 必须显式 capability**: 不写 capability = 全部 invoke 失败, 这是 Tauri 2.x 跟 v1 最大的 API 变化
2. **timing 日志用相对 + 绝对双标注**: 用户分不清"elapsed_ms 累计"和"阶段耗时", 标注 since_t0 + since_prev 一目了然
3. **不要相信 dev 模式体验 = 用户体验**: dev 模式 = 开发工具, 启动慢是按需编译的代价, 给开发者 hot reload; 用户实际跑 build/release
4. **白屏 = 用户感知"卡死"**: 启动 5s+ 没反馈 = 死机感, 必须有 splash/loading 反馈
5. **app.html 静态 splash ≠ 解决 dev 模式慢**: Vite 编译完前, webview 一直显示 Tauri 窗口 backgroundColor; 必须用官方 splash 方案 (双窗口, 静态 HTML 走 static/)
6. **避免一次性重构多个子模块**: 多个 P0/P1/P2 一起做, 容易混; 应该分批, 每批加回归测试
7. **Rust tracing 时间格式**: 0.3.23 没有 LocalTime, 升级会破坏 lockfile, 自己写 FormatTime impl 是最稳的

## 7. 测试基线 (本次未破坏)

- **前端 vitest**: 107/107 ✅
- **后端 cargo test**: 260/260 ✅
- **Tauri build**: 1m56s release, 10.3 MB binary ✅

## 8. 待办 (不在本次范围)

1. `qianxun_runtime` 重复 restore (RuntimeState::new 内部 + ensure_restored 又做一遍) — 留独立 PR
2. `ui.svelte.ts:17` 的 `sess_jwt_auth` fallback 字面量 — 跟 splash 一起改风险大, 留独立 PR
3. `tauri-plugin-splashscreen` 不可用 (Tauri 2.x 没官方) — 改用官方双窗口方案, 已是最终方案
4. 16 个 session 启动时 restore 重复日志 — 性能优化独立 PR
5. Vite a11y warnings (ThreeColumnLayout / TaskList / SettingsModal) — 不影响功能, 改 aria 留独立 PR
