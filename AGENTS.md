# AGENTS.md — Agent 工作约定（千寻项目）

面向自动化 agent（Claude Code、Codex、DSH agent 等）在此项目协作时的必读规则。
人类维护者可忽略本文件。

---

## 1. 临时文件统一放 `.tmp/`

**严禁** 在 cwd 根（`E:\develop\dsh-workspace\qianxun\`）下直接写临时脚本/日志/缓存。

- 临时目录：`<cwd>/.tmp/`
- 已 git 忽略（`.gitignore` `.tmp/` + `.tmp/**`）
- 已 Vite watch 忽略（`vite.config.ts` `server.watch.ignored` 追加 `**/.tmp/**`）
- cargo/tauri 不监听 `src-tauri/` 之外，天然隔离
- ESLint/Prettier 默认只扫源与配置，不扫 `.tmp/`

**用法**：`path.join('.tmp', 'probe-name.ts')`，文件命名加前缀或场景便于事后清理。
**清理**：会话结束可 `Remove-Item -Recurse .tmp/* -ErrorAction SilentlyContinue`（目录保留，README 解释用途）。

---

## 2. 进程终止判定（最严红线）

**绝对不能 kill 用户进程**。判定规则按"是否我启动的"严格收敛：

| 路径包含                                                                   | 状态                      | 动作        |
| -------------------------------------------------------------------------- | ------------------------- | ----------- |
| `C:\Users\maxu\AppData\Local\qianxun\qianxun.exe`                          | 用户安装版                | 永远不动    |
| `C:\Program Files (x86)\Microsoft\EdgeWebView\*\msedgewebview2.exe`        | 系统 Edge/WebView2 运行时 | 永远不动    |
| `C:\Windows\System32\*`                                                    | 系统进程                  | 永远不动    |
| `C:\Program Files\Tencent\Weixin\Weixin.exe` 等非项目应用                  | 第三方                    | 永远不动    |
| `E:\develop\dsh-workspace\qianxun\src-tauri\target\debug\qianxun.exe`      | 我启的 debug 二进制       | 我可以 kill |
| `E:\develop\dsh-workspace\qianxun\.tmp\` 下任何 node/cmd 子进程            | 我启的 vite/子进程        | 我可以 kill |
| `E:\develop\dsh-workspace\qianxun\src-tauri\target\debug\build\*` 下 cargo | 我启的 cargo              | 我可以 kill |

**操作前必查**：`Get-CimInstance Win32_Process -Filter "ProcessId=$id"` → 看 `CommandLine` / `ExecutablePath`。
`Path` 字段在 `Get-Process` 中可读，但要看真实可执行文件路径（`ExecutablePath` 更可靠）。

**绝不能仅凭"看起来像我的进程"** 就 kill——曾经的踩坑记：把 release `qianxun.exe`（路径 `C:\Users\maxu\AppData\Local\qianxun\`）误判为可杀，破坏用户生产数据。规则：**路径不在当前 cwd 链下，绝不动**。

---

## 3. 提交规范

- Conventional Commits（`feat` / `fix` / `refactor` / `chore` / `docs` / `style` / `test` / `perf` / `build` / `ci` / `revert!`）
- 中文 commit body（仓库已习惯此风格）
- body 引用探针脚本（`e2e/probe-*.ts`）时**已删除**的可以保留引用，注明"已删"
- 多关注点拆 commit：CDP 基建、bug 修复、交互优化 分笔提交，便于 revert

---

## 4. 调试基建现状

千寻 v0.5.0 起，**debug 构建内置 Playwright CDP**（`main.rs` 隔离 WebView2 user-data-dir + `tauri.debug.conf.json` 注入 `additionalBrowserArgs`）。使用方式：

```bash
# 终端 1
pnpm dev

# 终端 2（任何工作目录均可）
npx tsx e2e/probe-xxx.ts
```

CDP 端口固定 **10222**（release 默认 0，不暴露）。user-data-dir 隔离在 `%LOCALAPPDATA%\com.qianxun.desktop\EBWebView-dev\`，与 release 完全独立。

**完整经验文档**（踩坑 4 个深层坑）：`docs/07-Tauri调试基建与CDP经验.md`。

---

## 5. 项目命令速查

| 用途                           | 命令                                              | 端口 / 路径                                                           |
| ------------------------------ | ------------------------------------------------- | --------------------------------------------------------------------- |
| dev (HMR + debug 二进制 + CDP) | `pnpm dev`                                        | vite 5190 / qianxun debug / CDP 10222                                 |
| 前端 only dev                  | `pnpm dev:web`                                    | vite 5190                                                             |
| release build                  | `pnpm tauri build`                                | `src-tauri/target/release/qianxun.exe`                                |
| 全部检查                       | `pnpm all:check`                                  | prettier + eslint + svelte-check + vitest + cargo fmt + clippy + test |
| Rust 单测                      | `cargo test --manifest-path src-tauri/Cargo.toml` |                                                                       |
| e2e（spawn debug 二进制）      | `pnpm e2e:build && pnpm e2e`                      | fixture 自动 spawn，CDP 10222 保留                                    |

---

## 6. 项目结构要点

- `src/features/search/disk/DiskScan.svelte` — 磁盘扫描页（浅层模型 + 流式扫描）
- `src-tauri/src/disk.rs` — IPC 命令 + serde rename_all **必须包含 rename_all_fields**（见 docs/07）
- `src-tauri/crates/fff-core/src/disk.rs` — 扫描器，**前端契约只认浅层树**（`tree_root` 根带 children、子项 children=[]）
- `docs/` — 实战提炼文档（产品/架构/编码规范/Tauri 窗口/CDP 经验）
- `e2e/` — Playwright e2e 与 CDP 探针脚本
- `mobile/examples/` — 移动端 `mobile-access/` 的可复制示例（按场景分子目录）
- `.tmp/` — agent 调试临时文件

### 移动端 mobile-access 写法

- **用户态**写到 `<DSH_HOME>/mobile-access/custom.{css,js}`，**绝不让 agent 直接改这里**——这是用户数据目录，复制/删除由用户决定。
- **agent 提供的能力**写到 `mobile/examples/<scenario>/custom.{css,js}`，文档 (`docs/05-移动端定制指南.md`) 引导用户复制。
- 契约：`window.qxMobile.register(({ root }) => { ...; return 清理函数 })`（详见 docs/05 §3）。定制 UI 进 root，**不**直接改 DSH 节点结构（升级易碎、热替换无法回滚）。需要增强 DSH 元素时调它的公开方法（`scrollIntoView` / className / `addEventListener`）即可，**不**做 DOM 插入/移动。
