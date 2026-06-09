# 千寻 tauri→core 跑通指南

> 状态: 生效 | 最后更新: 2026-06-09 | 适用范围: Tauri 桌面 (Svelte 5) + qianxun-runtime (in-process)
>
> **设计基线**: [ADR-0003: 桌面 + ACP 同进程 2-Mode 互斥](../30_决策/ADR-0003_desktop_2mode.md)
>
> **本文件**只讲 tauri 桌面端如何端到端跑通。daemon / VPS Server 不在 "tauri → core" 主线,仅在 §6 简述。

## §0 前置条件

| 工具 | 最低版本 | 用途 |
|---|---|---|
| **Rust** | 1.85+ (rust-toolchain.toml 锁 stable) | qianxun-core + qianxun-runtime + Tauri Rust 端编译 |
| **Node.js** | 18+ (推荐 22) | SvelteKit dev/build |
| **pnpm** | 9+ | Tauri 前端包管理 (lockfile 是 `pnpm-lock.yaml`) |
| **Tauri CLI** | 2.0+ (`cargo install tauri-cli --version "^2.0"`) | `pnpm tauri dev` / `pnpm tauri build` 调它 |
| **Webview2** | Windows 10+ 已自带 | Tauri 在 Windows 渲染所需 |
| **LLM API key** | 任一: `DEEPSEEK_API_KEY` / `MINIMAX_API_KEY` | provider 鉴权 (走 Anthropic 协议) |

### §0.1 设置 API key

```powershell
# 走 deepseek
$env:DEEPSEEK_API_KEY = "sk-xxx"

# 走 minimax
$env:MINIMAX_API_KEY = "sk-xxx"
```

也可以写到 `~/.qianxun/config.json` 的 `providers.<name>.api_key` 字段,启动时自动读。

### §0.2 全局配置 (`~/.qianxun/config.json`)

```json
{
    "active_provider": "deepseek",
    "providers": {
        "deepseek": { "api_key": "sk-...", "model": "deepseek-v4-flash", "base_url": "https://api.deepseek.com/anthropic" },
        "minimax":  { "api_key": "sk-...", "model": "MiniMax-M3",        "base_url": "https://api.minimaxi.com/anthropic" }
    }
}
```

切换 active provider **不需要重启**:编辑 `active_provider` 字段后重启 desktop binary,或重新启 `pnpm tauri dev`。

API key 读取顺序(`qianxun-core/src/config.rs:398-421`):env 变量 → config.json → 空字符串(触发 `new_for_test()` fallback)。

## §1 编译

```powershell
cd E:\git\maxu\qianxun

# 1a. 编译所有 workspace crate
cargo build --workspace --release
# 产物: target/release/qx.exe (cli/acp/tui/daemon/server 多模式 binary)

# 1b. 编译 Tauri 桌面
cd qianxun-desktop
pnpm install
pnpm tauri dev      # dev 模式 (HMR + native window)
# 或
pnpm tauri build    # 生产构建 (7 平台: appimage/deb/rpm/nsis/msi/app/dmg)
```

> **Tauri 编译陷阱**: 首次 `pnpm tauri dev` 会下载 Tauri 2.x 依赖链 (~1.5GB) + 编译 webview2-com + windows-rs 等,耗时 5-10 分钟。后续增量编译 <10s。

## §2 启动 Tauri 桌面

```powershell
cd E:\git\maxu\qianxun\qianxun-desktop
$env:DEEPSEEK_API_KEY = "sk-xxx"
pnpm tauri dev
```

预期输出:
```
[setup] RuntimeState::new(config) - provider id=deepseek model=deepseek-v4-flash base_url=...
[setup] builtin tools registered=8
[setup] memory opened path=~/.qianxun/mem.db
[setup] skills loaded count=1
[setup] session store initialized at ~/.qianxun/daemon.db
[setup] connected
[webview] http://localhost:5173/
```

打开 Tauri 窗口,在 InputBox 输入消息 → 流式响应显示在 MessageBubble。

## §3 端到端链路

详见 [`desktop-state.md`](../10_事实源/desktop-state.md) "端到端链路" 段。简要 11 步:

```
1. ChatView button click
2. chat.svelte.ts:send() 追加 user msg
3. ipc/runtime.ts:sendMessage() invoke "send_message"
4. Tauri commands/runtime/send.rs:send_message
5. RuntimeApi::send_message → send_message_impl
6. tokio::spawn processing_loop::handle_user_message
7. mpsc::Receiver<SseEvent> 64 容量
8. spawn_event_emitter 消费 rx, app.emit("session_event")
9. Svelte onSessionEvent 全局 listener 路由
10. chat-stream.ts 12-event 状态机更新 MessageStreamState
11. sessionStore 反应式更新, MessageBubble 重渲染
```

每跳都序列化:仅跳 3 (Tauri IPC) 和跳 8 (SseEvent → JSON) 有序列化,其它都是 in-process Rust 函数调用或 mpsc 通道。

## §4 测试

```powershell
# Rust 后端
cd E:\git\maxu\qianxun
cargo test --workspace
# 期望: 248+ passed / 0 failed (基线, 2026-06-08)

# 前端
cd qianxun-desktop
pnpm test                                    # vitest
pnpm run check                               # svelte-check 类型检查

# 单模块
cargo test -p qianxun-runtime -- send_message_impl
cargo test -p qianxun-core -- agent
```

## §5 故障排查

### Q1: 启动 desktop 报 "DEEPSEEK_API_KEY not set"

**原因**: 环境变量缺失或 `~/.qianxun/config.json` 没有 `providers.<active>.api_key` 字段。

**解决**:
```powershell
$env:DEEPSEEK_API_KEY = "sk-xxx"
pnpm tauri dev
```

缺 key 时 `create_provider` fallback 到 `new_for_test()`,desktop 能启动但 `send_message` 必返 `LlmError::NoApiKey`。

### Q2: Tauri 第一次跑很慢

**原因**: 下载 Tauri 2.x 依赖链 (~1.5GB) + 编译 webview2-com + windows-rs。

**解决**: 耐心等 5-10 分钟,后续增量编译 <10s。

### Q3: `send_message` 返 `LlmError`

**原因**: API key 错误 / rate limit / upstream 错误。

**解决**:
- 检查 `~/.qianxun/config.json` `active_provider` 字段
- 切换 provider:改 `active_provider` 字段 + 重启 desktop
- 详细见 `runtime-state.md` "SseEvent 12 变体 / Error 错误码" 段

### Q4: Plan 列表看不到

**原因**: `list_plans` 在 RuntimeApi trait 里有,但 Tauri 无 command 包装,前端无 invoke 包装。

**解决**: 见 `desktop-state.md` "P0-3" 缺口。

### Q5: 重启 desktop 后 Plan 消失

**原因**: Plan 存储是 in-memory HashMap (`qianxun-runtime/src/state.rs:130`),不持久化。

**解决**: 见 `desktop-state.md` "P1-1" 缺口。

### Q6: SQLite "database is locked"

**原因**: WAL 模式下多 writer 冲突,或 `daemon.db` 同时被 desktop 和 daemon binary 持有。

**解决**:
- 确保 desktop 和 daemon 不同时跑
- 长期方案:desktop 改用 `desktop.db` 路径(见 P1-2)
- 临时:删 `~/.qianxun/daemon.db-wal` 和 `-shm` 文件(慎用,丢未 commit 数据)

### Q7: 强密码凭据丢失 (Tauri stronghold)

**原因**: iota_stronghold 加密的 `stronghold-snapshot.bin` 损坏或密码错。

**解决**:
- Tauri 模式下:`<app_local_data_dir>/stronghold-snapshot.bin` 是 ChaCha20 加密,密码错 `get_secret` 返 `Ok(None)`,可重设
- Web fallback 模式:localStorage base64 编码(仅脱敏,不真加密)

## §6 数据清理

```powershell
# 清所有 qianxun 持久化 (重置)
Remove-Item "$env:USERPROFILE\.qianxun\*" -Recurse -Force -ErrorAction SilentlyContinue

# 仅清数据库
Remove-Item "$env:USERPROFILE\.qianxun\mem.db*" -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.qianxun\daemon.db*" -ErrorAction SilentlyContinue

# 清配置
Remove-Item "$env:USERPROFILE\.qianxun\config.json" -ErrorAction SilentlyContinue

# 清 Tauri 用户态 (localStorage)
# 浏览器 DevTools → Application → Storage → Clear site data
# (在 Tauri 模式: WebView 工具类似操作)
```

## §7 daemon / VPS Server (可选, 不在 tauri→core 主线)

ADR-0003 之后,tauri 桌面走 in-process 调 runtime,不再走 HTTP → daemon。`qianxun` binary 的 `daemon` / `server` mode 仍存在但**不是桌面端的用户面**:

```powershell
# 可选: 启 daemon 给其他客户端用 (VPS 转发, 远程设备)
.\target\release\qx.exe --daemon --port 23900

# 可选: 启 VPS Server (控制面, 远端)
.\target\release\qx.exe --server --port 23901
```

如果只跑 tauri 桌面,**不需要启动 daemon 或 server**。`qianxun` binary 的 `--daemon` / `--server` 模式用于 VPS 远端场景,不在 "tauri → core" 范围。

## §8 文档索引

| 文档 | 位置 | 内容 |
|---|---|---|
| **本文件** | `docs/30_子项目规划/00-RUNNING-GUIDE.md` | tauri 桌面跑通指南 |
| 项目规则 | `CLAUDE.md` | 技术栈 + LLM Provider 配置 + 模块结构 |
| 当前决策 | `docs/30_决策/ADR-0003_desktop_2mode.md` | tauri+ACP 2-Mode 互斥 (唯一现行决策) |
| 共享契约 | `docs/30_子项目规划/_shared-contract.md` | RuntimeApi + SseEvent 契约 (v3) |
| Tauri + Runtime 集成 | `docs/30_子项目规划/04b-tauri-runtime-integration.md` | 当前 active 规划 |
| qianxun-runtime 抽取 | `docs/30_子项目规划/04c-qianxun-runtime-extraction.md` | runtime crate 抽取设计 |
| qianxun-runtime 状态 | `docs/10_事实源/runtime-state.md` | RuntimeApi 6 方法 + SseEvent 12 变体 |
| qianxun-desktop 状态 | `docs/10_事实源/desktop-state.md` | Tauri 10 command + 11 store + 端到端链路 |
| Tauri README | `qianxun-desktop/README.md` | Tauri 端命令 + P0 缺口 |
| 实施经验 | `docs/40_经验/2026-06-08_04b_subtask_{2,3,4}_*.md` | Tauri 集成实施记录 |

**TL;DR**:
- **编译**: `cargo build --workspace` + `cd qianxun-desktop && pnpm install`
- **启动**: `DEEPSEEK_API_KEY=sk-xxx pnpm tauri dev`
- **测试**: `cargo test --workspace` (248+ passed) + `pnpm test` (vitest)
- **当前阶段**: 4a-2 (用户手动 E2E 验收) — 详见 `desktop-state.md` "P0 缺口"
