# rtk（Rust Token Killer）内部机制与 DSH 接入可行性分析

> 对象仓库：`E:\git\ai\rtk`（只读分析，未做任何修改）
> 目的：为"rtk 能否接入 DSH（DeepSeek Harness / Cordis 插件架构的 agent 运行时）"提供事实依据
> 分析日期：2026-11-11
> 所有结论均带 `文件:行号` 证据；DSH 侧事实来自本次会话的 Cordis Inspect 实测（无文件路径，单独标注）

---

## 0. 一句话结论（TL;DR）

1. **rtk 的本质是一个「命令代理 + 输出过滤器」**，不是 agent 框架、不是 LLM 代理。它的可复用面只有两个：**改写命令**（`rtk rewrite`）和**过滤输出**（`rtk <cmd>` / `rtk pipe`）。
2. **`rtk rewrite` 是纯 argv 子进程接口**（不是 stdin JSON），退出码承载权限判定，是四个现成 agent 插件（OpenClaw / Hermes / Pi / Claude Code）共同依赖的唯一契约面。
3. **对 DSH 最现实的接入面是 (a) 调 `rtk rewrite`**——但有一个硬约束：**DSH 的 `tools/pre-execute` 只能 allow/deny/cancel/ask，无法替换工具入参**。要真正替换命令（`git status` → `rtk git status`），DSH 侧必须走「包装/替换 terminal 工具」或「包装 ToolExecutor」，而不是 pre-execute 钩子。
4. **DSH 侧更顺手的接入面反而是输出侧**：`tools/post-execute` 的 `accept` 分支允许**替换 result content**，正好对应 rtk 的 PostToolUse 压缩，且 `rtk pipe --filter <name>` 是纯 stdin→stdout、Apache-2.0 的最小接口。
5. **许可红线**：`rtk` CLI / `rtk-text` = Apache-2.0；`rtk-engine` / `rtk-plugin` / `rtk-proxy` = BUSL-1.1。**作为第三方 agent 运行时调用 `rtk rewrite`（子进程）不触碰 BUSL；链接 rtk-engine crate 则触发 BUSL 传染。**

---

## 1. 核心工作原理

### 1.1 目录结构

| 目录 | 职责 | 证据 |
|---|---|---|
| `src/lib.rs`（213KB） | CLI 入口（clap）：`Cli`/`Commands` 枚举、分发 | `src/lib.rs:86-104`、`src/lib.rs:1108` |
| `src/main.rs`（242B） | 只是 `main()`，调用 `rtk::entry()` | `src/main.rs`（全文 242B） |
| `src/cmds/` | **按生态分的命令过滤器**（12 个生态） | `src/cmds/README.md:5-11` |
| `src/core/` | 共享基础设施（44 个模块） | `src/core/mod.rs:3-44` |
| `src/hooks/` | hook 安装/校验/信任 + 权限判定 + `rtk rewrite` CLI 桥 | `src/hooks/README.md:5-16` |
| `src/discover/` | **改写规则注册表**（热路径）+ 历史分析 | `src/discover/README.md:7-11` |
| `src/parser/` | 输出解析器/格式化器 | `src/parser/mod.rs` |
| `src/analytics/` | `rtk gain` 等统计面板 | `src/analytics/mod.rs` |
| `src/learn/` | 命令学习/检测 | `src/learn/mod.rs` |
| `src/text/`（crate） | 文本原语，**Apache-2.0** | `text/Cargo.toml:9` |

`src/cmds/` 的 12 个生态（各自有 README）：
`cloud`（aws/docker/kubectl/curl/wget/psql）、`dotnet`、`git`（git/gh/glab/gt/diff）、`go`、`js`（npm/pnpm/vitest/tsc/eslint/playwright/prisma…）、`jvm`（mvn/gradlew）、`php`、`python`（pytest/ruff/mypy/sqlfluff/uv）、`ruby`、`rust`（cargo）、`scala`（sbt）、`system`（ls/tree/read/grep/find/wc/env/json/log/deps/summary/format/pipe/ast-grep/ctest）。清单见 `src/cmds/README.md:26-36`。

### 1.2 「执行原命令 → 捕获 stdout/stderr → 过滤/压缩 → 打印」——**确认成立**

组件边界定义（`src/cmds/README.md:5`）：

> "Every module here calls an external CLI tool (`Command::new("some_tool")`), transforms its stdout/stderr to reduce the bytes the agent reads, and records the reduction via `core/tracking`."

准入规则（`src/cmds/README.md:11`）：

> "a module belongs here if and only if it executes an external command and filters its output."

执行骨架集中在 `src/core/runner.rs`：
- `run()` / `run_inner()`：`src/core/runner.rs:574-648`
- `run_filtered()`：`src/core/runner.rs:650-667`（`F: Fn(&str) -> String`，纯函数式过滤器）
- `run_filtered_with_exit()`：`src/core/runner.rs:669-686`
- `run_passthrough()`：`src/core/runner.rs:688-702`
- `run_streamed()`：`src/core/runner.rs:704+`

四种 `FilterMode`（`src/cmds/README.md:65-72`）：

| FilterMode | 行为 | 入口 |
|---|---|---|
| `CaptureOnly` | 全量缓冲 stdout，事后整体交 `filter_fn`；stderr 实时透传 | `run_filtered()` 默认 |
| `Buffered` | 缓冲 stdout→过滤→打印；stderr 实时 | stdout-only 路径 |
| `Streaming` | 逐行喂 `StreamFilter::feed_line()`，即时输出 | `run_streamed()` |
| `Passthrough` | 直接继承父 TTY，零缓冲 | `run_passthrough()` |

五阶段（`src/cmds/README.md:83-89`）：Spawn → Filter → Print → Track → Exit code。
退出码契约（`src/cmds/README.md:228-246`）：模块统一返回 `Result<i32>`，`main.rs` 单点 `process::exit(code)`；`Ok(N)` = 被代理命令的退出码，`Err(e)` = rtk 自身失败（exit 1）。**模块永不自己 `process::exit()`。**

### 1.3 `src/core/` 各模块（`src/core/mod.rs:3-44`）

| 模块 | 行数级 | 作用 |
|---|---|---|
| `runner.rs` | 96KB | 执行骨架（见上） |
| `tracking.rs` | 168KB | SQLite 记账：raw vs filtered token 节省；表 `hook_decisions` / `commands` / `parse_failures`（`src/core/tracking.rs:498-516`） |
| `toml_filter.rs` | 76KB | **TOML DSL 过滤器引擎**（`filters.toml`） |
| `utils.rs` | 66KB | `resolved_command`、`ChildArgExt::child_args` 等 |
| `arg_tokenizer.rs` | 61KB | **argv 层**参数解析（flag 是否带值） |
| `stream.rs` | 51KB | `run_streaming()`、`FilterMode`、`StreamFilter`、`RAW_CAP` |
| `pipeline_cmd.rs` | 90KB | `rtk pipeline show/validate/explain/bench/...` |
| `pipeline_config.rs` | 49KB | 加载/解析/信任 pipeline 文件；`before_shell_decision()` |
| `retriever.rs` | 54KB | 「召回库」：被截断的输出存 sqlite，agent 用 `rtk recall <hash>` 取回 |
| `filter.rs` | 28KB | **源码注释/样板剥离**（`FilterLevel`: None/Minimal/Aggressive，`src/core/filter.rs:8-12`；`FilterStrategy` trait `src/core/filter.rs:37-39`） |
| `tee.rs` / `tee_file.rs` | 13/15KB | 旧版文件式恢复（已被 `[retriever]` 取代，见 `src/core/config.rs:94-116`） |
| `shell.rs` | 23KB | shell 方言（`ShellDialect`，fish vs POSIX） |
| `truncate.rs` | 2KB | 全局截断上限常量 `CAP_ERRORS`/`CAP_WARNINGS`/`CAP_LIST`/`CAP_INVENTORY` |

**注意层级**：`arg_tokenizer.rs` 是 **argv 层**（已切分的 argv → flag/value），`discover/lexer.rs`（80KB）是 **shell 字符串层**（引号/转义/操作符）。两者刻意分层（`src/discover/README.md:44-46`）。

### 1.4 改写注册表（真正的「映射表」）

- 规则定义在 `src/discover/rules.rs`：`pattern`（正则）、`rtk_cmd`（如 `"rtk cargo"`）、`rewrite_prefixes`、`category`、`savings_pct`（`src/discover/README.md:111-119`）
- 注册表入口：`src/discover/registry.rs:768-776` `rewrite_command(cmd, excluded, transparent_prefixes) -> Option<String>`
- 改写算法四步（`src/discover/README.md:28-42`）：剥尾部重定向 → 特殊形态短路（`head -20 f` → `rtk read f --head-lines 20`）→ 分类（剥 env 前缀、归一路径、剥 git 全局选项）→ 替换前缀并复原前后缀
- 复合命令：`&&`/`||`/`;`/`|`/`|&`/`&` 逐段改写；管道中间级保持原样，只有 pipeline-safe 的末级改写（`src/discover/README.md:24`）
- 保护条件（`src/discover/README.md:36-40`）：`RTK_DISABLED=1`、`gh --json/--jq/--template`、`cat` 带非 `-n` 参数、`cat/head/tail` 带 `>`/`>>`、命中 `hooks.exclude_commands`

---

## 2. `rtk rewrite` 命令（最关键的可复用面）

### 2.1 输入契约：**argv，不是 stdin JSON**

CLI 定义（`src/lib.rs:1096-1101`）：

```rust
/// Rewrite a raw command to its RTK equivalent (single source of truth for hooks)
Rewrite {
    /// Raw command to rewrite (e.g. "git status", "cargo test && git push")
    /// Accepts multiple args: `rtk rewrite ls -al` is equivalent to `rtk rewrite "ls -al"`
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
},
```

分发（`src/lib.rs:3726-3730`）：

```rust
Commands::Rewrite { args } => {
    let cmd = args.join(" ");
    hooks::rewrite_cmd::run(&cmd)?;
    0
}
```

**结论**：`rtk rewrite "git status"`（或 `rtk rewrite git status`，argv 用空格 join）。**没有 stdin JSON 协议**——这一点与 `rtk hook <agent>`（读 stdin JSON）截然不同，是两个不同的入口。

### 2.2 输出契约：stdout 打印改写后的命令，**退出码承载权限判定**

`src/hooks/rewrite_cmd.rs:91-125` 是全部实现，核心 12 行：

```rust
let verdict = permissions::check_command(cmd);
let decided = decision::ApprovalOwner::from_env().apply(
    decision::decide(cmd, verdict, permissions::DenyRules::Unattested),
    verdict,
);
if !matches!(decided, HookDecision::Deny) { track_tee_read(cmd); }
match decided {
    HookDecision::AllowRewrite(rewritten) => { print!("{}", rewritten); flush; Ok(()) }
    HookDecision::AskRewrite(rewritten, _) => { print!("{}", rewritten); flush; exit(3) }
    HookDecision::Deny  => exit(2),
    HookDecision::Defer => exit(1),
}
```

### 2.3 退出码语义（权威表）

`src/hooks/rewrite_cmd.rs:70-77`（函数 doc）与 `src/lib.rs:1067-1095`（CLI doc）、`docs/usage/FEATURES.md:1299-1314`：

| 退出码 | stdout | 含义 | 集成方应做 |
|---|---|---|---|
| **0** | 改写后的命令 | allow 规则覆盖了命令链上每一条 → 可自动放行 | 直接用 stdout 替换命令 |
| **1** | 无 | 不改写：无 RTK 等价物 / 无内容可改 / config 排除 / heredoc / `$(...)` / 文件重定向 / shell wrapper 内命中 deny | 原样执行 |
| **2** | 无 | 命中 deny 规则 | 拦截或交给宿主自身 deny |
| **3** | 改写后的命令 | 命中 ask 规则，或 allow 规则未覆盖全部命令（含**完全没有规则**的情形） | 用 stdout 替换命令，但**要提示用户** |

实证（`docs/usage/FEATURES.md:1310-1314`）：
```
rtk rewrite "git status"      # -> "rtk git status"  (exit 3 with no rules)
rtk rewrite "echo hello"      # -> (exit 1, no rewrite)
rtk rewrite "rtk git status"  # -> "rtk git status"  (exit 3 with no rules, unchanged)
```

### 2.4 失败语义细节（第三方集成必须知道）

- **stderr**：正常路径不写 stderr（`src/hooks/rewrite_cmd.rs:111-124` 只有 stdout 与 exit code）。错误信息在 `rtk hook check` 路径才有（`src/lib.rs:3714-3721`）。
- **安全阀**：`Default`（无规则匹配）必须映射到 exit **3** 而非 0，否则会绕过最小权限默认。见 `src/hooks/rewrite_cmd.rs:215-226`（注释引用 issue #1155）与 `src/hooks/rewrite_cmd.rs:237-245` 的 `expected_exit_code()`。
- **超时**：rtk 自己不定超时，由调用方定。现有 delegate 的取值：OpenClaw 2000ms（`openclaw/index.ts:80`）、Pi 2000ms（`hooks/pi/rtk.ts:20,46-49`）、Hermes 5000ms（`hooks/hermes/rtk-rewrite/__init__.py:16,43-48`，可用 `RTK_HERMES_TIMEOUT` 覆盖）。
- **版本要求**：`rtk rewrite` 自 0.23.0 引入（`hooks/claude/rtk-rewrite.sh:26-45`、`hooks/pi/rtk.ts:21,106-113`）。旧二进制要优雅降级（exit 0 放行）。
- **`RTK_REWRITE_HOST` 环境变量**（`src/hooks/decision.rs:84`）：让 delegate 声明「审批归我管」。目前只有 `openclaw` 解析为 `ApprovalOwner::Delegate`（`src/hooks/decision.rs:104-131`）。效果：把 `Default` 判定的 exit 3 降级为 exit 0（`src/hooks/decision.rs:144-153`），**但显式 ask 规则仍是 3，deny 仍是 2**。
  - 为什么用环境变量而非 CLI flag：`Commands::Rewrite` 是 `trailing_var_arg=true, allow_hyphen_values=true`，一个不认识 `--host` 的旧 rtk 会把它折进命令文本，deny 规则就匹配不上了（`src/hooks/decision.rs:71-83`）。
  - 不拥有审批权的调用方**必须 scrub 掉这个变量**（`hooks/claude/rtk-rewrite.sh:55-59` 用 `env -u RTK_REWRITE_HOST`）。

### 2.5 哪些 agent 走 `rtk rewrite`（子进程）vs 走 `rtk hook <agent>`（进程内）

`src/hooks/README.md:5`：

> "External scripts and plugins are thin delegates to `rtk rewrite`; native processors such as Codex call the same Rust rewrite registry directly."

| 走 `rtk rewrite` 子进程 | 证据 |
|---|---|
| Claude Code（bash hook） | `hooks/claude/rtk-rewrite.sh:59` |
| OpenClaw（TS 插件） | `openclaw/index.ts:85` |
| Hermes（Python 插件） | `hooks/hermes/rtk-rewrite/__init__.py:62-69` |
| Pi / Oh My Pi（OMP）（TS 扩展，共用同一文件） | `hooks/pi/rtk.ts:46`；OMP 通过 `legacy-pi-compat` 装同一文件（`src/hooks/README.md:37`） |

| 走 `rtk hook <agent>` 进程内 | 证据 |
|---|---|
| Claude / Trae / Codex / Cursor / Gemini / Copilot / Droid / Antigravity / Vibe / Opencode | `src/lib.rs:1275-1313`（`HookCommands` 枚举） |

`rtk hook opencode` 的输出是 JSON：`{"command": "rtk git status"}`（`hooks/README.md:290-299`）。

### 2.6 决策核心：`src/hooks/decision.rs`

- 权限优先级：`Deny > Defer > Ask > Allow(explicit) > Default(=ask)`（`src/hooks/README.md:75`）
- `decide()` 的四道门（`src/hooks/decision.rs:156-190`）：
  1. **Deny 优先**（`:253-256`）——shell wrapper 内命中 deny → `Defer`（不 rewrite、不决定）
  2. **fish 脚本 wrap**（可选，`hooks.wrap_fish_scripts`，默认 false）
  3. **拒绝不可证明安全的构造**：命令替换 / 文件重定向 / heredoc（`:173-181`）
  4. **有规则才 rewrite**；只有显式 `Allow` 才自动放行，`Default` 一律 `AskRewrite`（`:182-186`）
- 规则来源：Claude Code 的 `settings.json`（项目 + 全局 + `.local` 变体），只取 `Bash(...)` 规则（`src/hooks/README.md:78`）

---

## 3. 配置系统：`config.toml`

### 3.1 位置与结构

路径解析：`src/core/config.rs:531-534` → `user_dirs::config()/config.toml`
（Linux `~/.config/rtk/`，macOS `~/Library/Application Support/rtk/`，Windows `%APPDATA%\rtk\`，见 `docs/guide/pipelines/overview.md:93`）

顶层结构（`src/core/config.rs:10-34`）：

```rust
pub struct Config {
    pub tracking: TrackingConfig,       // src/core/config.rs:168-184
    pub display: DisplayConfig,         // src/core/config.rs:186-208
    pub filters: FilterConfig,          // src/core/config.rs:210-230
    pub retriever: RetrieverConfig,     // src/core/config.rs:19
    tee: Option<LegacyTeeConfig>,       // src/core/config.rs:73-81（已废弃，自动迁移）
    pub telemetry: TelemetryConfig,     // src/core/config.rs:232-239
    pub hooks: HooksConfig,             // src/core/config.rs:118-166
    pub limits: LimitsConfig,           // src/core/config.rs:241-265
    pub awareness: AwarenessConfig,     // src/core/config.rs:67-71
}
```

完整 TOML 形态见 `docs/guide/getting-started/configuration.md:24-64`。

### 3.2 `[hooks]` 段（`src/core/config.rs:118-166`）

| 字段 | 类型 | 语义 |
|---|---|---|
| `exclude_commands: Vec<String>` | 列表 | 不自动改写的命令。匹配的是**剥掉 env 前缀后的完整命令**；子命令模式可用（`"git push"` 排除 `git push origin main`）；`^` 开头当正则，非法正则回退前缀匹配（`docs/guide/getting-started/configuration.md:142-186`） |
| `transparent_prefixes: Vec<String>` | 列表 | 透明剥掉再复原的前缀，如 `docker exec mycontainer`（`src/core/config.rs:125-145`） |
| `suppress_hook_warning: bool` | bool | 只压制「没装 hook」警告，不压「hook 版本旧」提示（`src/core/config.rs:146-151`） |
| `wrap_fish_scripts: bool` | bool | 默认 false；开启后把 fish 脚本包成 `rtk run --shell fish -c '<script>'`（`src/core/config.rs:152-165`） |

环境变量 `RTK_SUPPRESS_HOOK_WARNING` 是三方覆盖（1/true/yes/on 强制开，0/false/no/off 强制关，其余回落 config）（`src/core/config.rs:319-348`）。

### 3.3 `[awareness] level`（`src/core/config.rs:36-71`）

| 值 | agent 被告知什么 | 何时用 |
|---|---|---|
| `default` | 只讲「怎么读压缩输出」，**不提 rtk 存在** | 有 hook 的 agent，rtk 保持隐形 |
| `high` | `default` + rtk 是什么 + `rtk gain`/`rtk bypass`/`RTK_DISABLED=1`/`rtk discover` | 想让 agent 主动问节省 |
| `full` | `high` + 「外部程序一律加 `rtk` 前缀」 | **无 hook 的 agent**，或想让 agent 自己驱动 rtk |

- 默认值 `default`（`src/core/config.rs:44-48`）；非法值是**解析错误**（`src/core/config.rs:590-597`）
- 无 hook 的 agent（Codex CLI、Cline、Windsurf、Kilo Code、Kimi）**永远**拿 `full`（`docs/guide/getting-started/configuration.md:86-87`；`hooks/README.md:13-16`）
- 对应 pipeline 里的 `[override.rtk-awareness] level = "full"`（`docs/guide/pipelines/overview.md:112`）

### 3.4 其它段

- `[tracking]`：`enabled`(默认 true) / `history_days` / `database_path`（`src/core/config.rs:168-184`）
- `[display]`：`colors` / `emoji` / `max_width`(120) / `pro_hint`（`src/core/config.rs:186-208`）
- `[filters]`：`ignore_dirs`（默认 .git/node_modules/target/__pycache__/.venv/vendor）/ `ignore_files`（默认 `*.lock`/`*.min.js`/`*.min.css`）（`src/core/config.rs:216-229`）——只作用于**读文件类**命令（ls/find/grep/read）
- `[limits]`：`grep_max_results`(200) / `grep_max_per_file`(25) / `status_max_files`(15) / `status_max_untracked`(10) / `passthrough_max_chars`(2000)（`src/core/config.rs:255-264`）
- `[retriever]`：召回库。`mode = "sqlite"(默认) | "tee" | "disabled"`，`max_entry_bytes`(10MiB) / `max_entries`(200) / `retention_days`(30) / `compression`(true) / `tee_max_files`(20) / `tee_max_file_size`(1MiB) / `tee_on_success`(false)（`docs/guide/getting-started/configuration.md:42-52, 130-140`）。旧 `[tee]` 段自动迁移（`src/core/config.rs:94-116, 375-406`）
- `[telemetry]`：`enabled`（默认 false）/ `consent_given` / `consent_date`（`src/core/config.rs:232-239`）

### 3.5 **pipeline 相关配置不在 `config.toml` 里**

`docs/guide/pipelines/writing-pipelines.md:31`：

> "`config.toml` is unchanged and separate: preferences (`[tracking]`, `[display]`, `[hooks]`, `[retriever]`, `[telemetry]`, `[limits]`, `[awareness]`) stay there. The pipeline holds behaviour; it never replaces an existing RTK file."

---

## 4. Pipelines：TOML 声明式管道

### 4.1 是什么

`docs/guide/pipelines/overview.md:8-12`：

> "RTK saves tokens by intercepting what your coding agent reads and sends. In v1.0 what it does is a **pipeline**: a file that says *when* RTK acts, *on what*, and *what it does*. Every built-in behaviour is written in that same grammar... The file is agent-agnostic. You write `shell`, `read`, `before_tool`; RTK translates to Claude Code, Cursor, Gemini CLI or whatever you set up with `rtk init`."

**重要**：`docs/guide/pipelines/overview.md:14-16` 注明 —— 「pipeline engine 在 Apache-2.0 版中不存在」（见 §9）。

### 4.2 六个内置 pipeline（编译进二进制，只读）

`rtk:default`、`rtk:posthook`、`rtk:proxy`、`rtk:delegate-reads`、`rtk:verbatim-diffs`、`rtk:terse`
（`docs/guide/pipelines/writing-pipelines.md:16`；实体文件在 `engine/builtin/*.toml`：`default.toml`、`posthook.toml`、`proxy.toml`、`delegate-reads.toml`、`terse.toml`、`verbatim-diffs.toml`）

### 4.3 文件位置与加载顺序

`docs/guide/pipelines/writing-pipelines.md:14-27`：

| 文件 | 作用域 | 备注 |
|---|---|---|
| `~/.config/rtk/pipeline.toml` | 用户 | 最先加载 |
| `~/.config/rtk/pipelines/*.toml` | 用户 | 全部加载，按文件名字母序 |
| `.rtk/pipeline.toml` | 项目 | **信任门控**——`rtk init` 会展示内容并询问；文件改动后要重新信任 |
| `.rtk/pipelines/*.toml` | 项目 | 全部加载，逐个信任门控 |

加载顺序：内置 → 用户 pipeline.toml → 用户 pipelines/ → 项目 pipeline.toml → 项目 pipelines/。后者覆盖前者。

### 4.4 规则树与语法

树形结构（`docs/guide/pipelines/writing-pipelines.md:35-50`）：

```text
schema_version = 1
[pipeline]                         # name, extends, enabled

[[hook.before_tool.<subject>]]     # 工具运行前
[[hook.after_tool.<subject>]]      # 工具运行后
[[proxy.request.<part>]]           # 请求发往 provider 前

[[directive]]                      # 注入给 agent 的文本
[method.<name>]                    # 链、内置方法的选项
[classifier.<name>]                # AI 判定器
[task.<name>]                      # 委派命令
[upstream.<name>]                  # 模型端点
[override.<name>]                  # 补丁：改继承来的规则
[guards]                           # 全局安全上限
```

规则解剖（`docs/guide/pipelines/writing-pipelines.md:123-143`）：

```toml
[[hook.after_tool.shell]]
name     = "term-default"
when     = { is_rtk_command = false }
use      = ["term"]
continue = false
enabled  = true
shadow   = false
```

| 键 | 必需 | 说明 |
|---|---|---|
| `name` | 是* | 全局唯一；*无歧义时默认取 `use` 首项 |
| `when` | 否 | 条件，全部满足才匹配；省略 = 全匹配 |
| `use` / `action` | 二选一 | `use` = 方法链；`action` = 控制流 |
| `continue` | 否 | 默认 `false`（首个匹配即胜）；`true` 继续评估后续规则 |
| `enabled` / `shadow` | 否 | 禁用 / 只算不发（影子模式） |

`site × event × subject` 三段定位（`docs/guide/pipelines/writing-pipelines.md:62-102`）：
- **site**：`hook` | `proxy`
- **event**：`before_tool` | `after_tool` | `request`
- **subject**（hook）：`read` `write` `edit` `shell` `search` `web` `subagent` `mcp` `any`
- **part**（proxy）：`tool_result` `tool_use` `assistant` `cache` `any`

### 4.5 「能不能用 TOML 声明『某命令 → 某种过滤方式』而不写 Rust？」

**分三种情况，答案不同：**

#### (i) 「某命令 → 换成 rtk 等价命令」——**可以，且就是内置行为**

`action = "rewrite"`（`docs/guide/pipelines/writing-pipelines.md:167`）：

> "**`rewrite`** — prefix the command with `rtk` when RTK has a filter for it (`ls` → `rtk ls`); leave it untouched otherwise. It never takes a target or options."

真实例子（`engine/builtin/default.toml:16-18`）：

```toml
# `ls` -> `rtk ls` for every command RTK has a filter for; others untouched
[[hook.before_tool.shell]]
name   = "rtk-rewrite"
action = "rewrite"
```

**但注意**：这条规则是「**凡是 rtk 有 filter 的命令**都加前缀」，**不是**一张用户可写的「命令 → 过滤方式」映射表。用户能改的只是它的开关（`[hooks] exclude_commands`、`transparent_prefixes`，仍在 config.toml，见 `writing-pipelines.md:167`）。真正的「命令 → rtk 子命令」映射表在 Rust：`src/discover/rules.rs`（`src/discover/README.md:111-119`）。

#### (ii) 「某类输出 → 某种压缩链」——**可以**，但需要 engine（BUSL）+ after_tool hook

`engine/builtin/posthook.toml:16-45` 全文即真实例子：

```toml
[[hook.after_tool.shell]]
name = "term-default"
when = { is_rtk_command = false }
use  = ["term"]

[[hook.after_tool.read]]
name = "lockfiles"
when = { basename_in = ["package-lock.json", "Cargo.lock", "yarn.lock", "pnpm-lock.yaml"] }
use  = ["lockfile"]

[[hook.after_tool.search]]
name = "grep-group"
when = { format = "matches" }
use  = ["matches"]

[[hook.after_tool.web]]
name = "web-default"
use  = ["web"]
```

可用方法清单在 `engine/src/methods/`：`ansi`、`base64_elide`、`cap`、`dedup`、`grep_group`、`lockfile`、`md_slim`、`minify_json`、`progress`、`rle`、`shape`、`strip_line_numbers`、`toon`、`truncate`、`web_md`（文件清单 `engine/src/methods/*.rs`；注册表 `engine/src/registry.rs`；`Method` trait `engine/src/method.rs`）。

**门槛**：需要 `rtk-engine` crate（BUSL-1.1，见 §9）+ `rtk hook` 侧支持 after_tool 的 agent。

#### (iii) 「某命令 → 行级剥离规则」——**可以，纯 TOML，且 Apache-2.0 就够**

这是另一套东西：**TOML filter DSL**（`filters.toml`），与 pipeline.toml 无关。DSL 见 `src/filters/README.md:29-62`：

```toml
[filters.my-tool]
description = "Short description of what this filter does"
match_command = "^my-tool\\b"          # regex matched against the full command string
strip_ansi = true
strip_lines_matching = [ "^\\s*$", "^noise pattern" ]
max_lines = 40
on_empty = "my-tool: ok"

[[tests.my-tool]]
name = "descriptive test name"
input = "raw command output here"
expected = "expected filtered output"
```

字段全集（`src/filters/README.md:47-62`）：`description` / `match_command` / `strip_ansi` / `filter_stderr` / `strip_lines_matching` / `keep_lines_matching` / `replace` / `match_output` / `truncate_lines_at` / `max_lines` / `tail_lines` / `on_empty`。

- 位置：项目 `.rtk/filters.toml` / 用户 `~/.config/rtk/filters.toml`（`src/filters/README.md:128-130`）
- 查找优先级：项目 → 用户 → 内置（编译进二进制）→ 透传；先匹配先赢（`src/filters/README.md:102-116`）
- 运行时 8 阶段流水线：`strip_ansi → replace → match_output → strip/keep_lines → truncate → tail_lines → max_lines → on_empty`（`src/filters/README.md:95`）
- **信任门控**：自定义 filter 文件**不被信任就不生效**（且在命令路径上**静默跳过**）；信任绑定文件内容 SHA-256，改了要重新 `rtk trust`（`src/filters/README.md:124-141`）
- 内置 68 个 filter 在 `src/filters/*.toml`（如 `terraform-plan.toml`、`ruff`… 实际是 `brew-install.toml`、`kubectl`… 全量清单见该目录）

**结论**：如果第三方 agent 想要「自己声明某命令怎么过滤」，**`filters.toml` 是最干净、最 Apache-2.0 的路径**——但它只能做**行级剥离**，不能做结构化重排（那是 Rust filter 或 engine method 的事，`src/cmds/README.md:13-16` 给了 TOML-vs-Rust 决策表）。

---

## 5. `engine/` 与 `proxy/`（BUSL-1.1）

### 5.1 `engine/` = 管道引擎（压缩引擎），**不是** HTTP 代理

`engine/README.md:3`：

> "The RTK pipeline engine: it parses, resolves and validates `pipeline.toml`, evaluates rules, and runs the methods and chains those rules name. The `rtk` CLI links it for the hooks; the `rtk-proxy` daemon links it for `proxy.request` rules."

结构：

| 路径 | 作用 |
|---|---|
| `engine/src/pipeline/schema.rs`（37KB） | pipeline.toml 的 schema |
| `engine/src/pipeline/resolve.rs`（46KB） | 继承/合并/解析 |
| `engine/src/pipeline/validate.rs`（75KB） | 校验 + 诊断码（W001…W023、E001…E023） |
| `engine/src/pipeline/eval.rs`（22KB） | 条件求值 |
| `engine/src/pipeline/conditions.rs`（16KB） | `when` 条件集 |
| `engine/src/pipeline/builtins.rs` | 内置 pipeline 装配 |
| `engine/src/methods/*.rs`（15 个） | 压缩方法实现 |
| `engine/src/method.rs` / `registry.rs` | `Method` trait 与注册表 |
| `engine/src/harness.rs`（46KB） | agent 工具名 → subject 的映射（`profile("claude-code")`，见 `src/hooks/hook_cmd/claude_pipeline.rs:106-109`） |
| `engine/src/block.rs` | `Block`/`Subject`/`Provenance`/`ContentFormat` 数据模型 |
| `engine/src/task.rs` | 任务 kind（`summarize`/`digest`/`generate`） |
| `engine/src/strategy.rs`、`chain.rs`、`shape.rs`、`process.rs`、`directive.rs`、`marker.rs` | 策略/链/形状/流程/指令/标记 |
| `engine/builtin/*.toml` | 六个内置 pipeline 的字面文件 |

**定位回答**：「engine 是压缩引擎？」→ **是的**。它把「一段文本 + 一个 subject」变成「压缩后的文本」，规则由 `pipeline.toml` 声明。

### 5.2 `proxy/` = 本地守护进程，拦截 **LLM provider 的 API 流量**

`proxy/README.md:14-33`：

> "`rtk` optimizes **command output** before it reaches your assistant's context. `rtk-proxy` brings the same engine to the **API traffic** itself: it sits between your AI coding tool and the real provider endpoint, runs the pipeline on the traffic — compression, and the relay of calls your pipeline routes to cheaper models — then relays the response back. It is agent-agnostic by design: it speaks the provider's dialect, not the agent's. Today Claude Code is wired, through `rtk claude`."

```
  Claude Code ──▶ rtk-proxy ──▶ api.anthropic.com
                     │
                     ├─ normalize: strip ANSI / redraws / dupes
                     ├─ your proxy.request.tool_result method rules
                     ├─ (opt-in) cache-align
                     └─ record usage ──▶ rtk gain
```

**工作模式回答**：
- **不是**「监听一个本地 HTTP 端口让 agent 来连」。它是**上游 provider 端点的中间件**——agent 通过它发请求，它转发给 `api.anthropic.com`（`proxy/README.md:27`）。
- 由 `rtk` CLI 拉起（`rtk claude` 启动守护进程再拉起 Claude；`rtk llm-proxy status` 查状态；`rtk gain` 看节省），见 `proxy/README.md:45-49`。
- **四件事**：intercept / compress / forward / record（`proxy/README.md:24`）
- 关键模块：`proxy/src/server.rs`（HTTP 服务端）、`forward.rs`（转发）、`ccr_forward.rs`（Claude Code Router 转发）、`pipeline/ingest_filter.rs`（73KB，请求摄取过滤）、`pipeline/ccr/*`（Anthropic/OpenAI 两种方言）、`pipeline/normalize.rs`、`pipeline/cache_align.rs`、`pipeline/redact.rs`、`pipeline/image_opt.rs`、`pipeline/pixel_compress.rs`、`pricing.rs`、`memory.rs`、`metrics.rs`、`store.rs`、`tokenizer.rs`
- **与 rtk CLI 完全解耦**：`proxy/README.md:52-54` —— "The crate lives in the `rtk` repository as its own workspace, and is **not** linked into the `rtk` CLI — `rtk` locates and spawns this binary."；`Cargo.toml:6-13` 明确 `proxy/` **不是** workspace member（只是为了不让 tokio/axum 进 CLI 依赖树）
- agent 无关性：「it speaks the provider's dialect, not the agent's」（`proxy/README.md:19-21`）—— 今天只接通了 Claude Code，其他 agent 待验证

---

## 6. `plugin/` 与 `openclaw/`

### 6.1 `plugin/` = **Rust 插件 SDK crate**（`rtk-plugin`，BUSL-1.1）

`plugin/Cargo.toml`：`name = "rtk-plugin"`, `license = "BUSL-1.1"`, `publish = false`, 依赖 `rtk-engine`（`plugin/Cargo.toml:8,12`）

`plugin/src/lib.rs:1-8` 重新导出 engine 的扩展面：

```rust
pub use rtk_engine::block::{Block, ContentFormat, Layer, Origin, Provenance, Subject};
pub use rtk_engine::method::{CostClass, Method, MethodCtx, MethodInfo, MethodKind, Outcome, Produces, Recovery};
pub use rtk_engine::registry::register;
pub use rtk_engine::task::{CorpusFile, TaskIntent, TaskKind, TaskOutcome, WorkerRequest, register_kind};
```

注册宏 `rtk_plugin!`（`plugin/src/lib.rs:10-33`）：

```rust
rtk_plugin!(methods: [MyMethod], kinds: [MyTaskKind]);
// 或简写
rtk_plugin!(MyMethod);
```

用途：写新的 **compression method** 和 **task kind**（`docs/guide/pipelines/plugins.md`；`engine/README.md:20`）。测试见 `plugin/tests/compression_method.rs`、`plugin/src/lib.rs:35-132`。

**注意**：这是 **Rust crate 级别**的插件，不是「agent 插件」。而且它必然拖进 BUSL。

### 6.2 `openclaw/` = OpenClaw 的 agent 插件（**最接近「agent 插件式接入」的样板**）

文件：`openclaw/index.ts`（208 行）、`openclaw/openclaw.plugin.json`、`openclaw/package.json`、`openclaw/README.md`

#### 完整实现逻辑（`openclaw/index.ts`）

**注册**（`:110-124`）：

```ts
export default function register(api: any) {
  const pluginConfig = api.config ?? {};
  const enabled = pluginConfig.enabled !== false;
  const verbose = pluginConfig.verbose === true;
  if (!enabled) return;
  if (!checkRtk()) { console.warn("[rtk] rtk binary not found in PATH — plugin disabled"); return; }

  api.on("before_tool_call", (event: { toolName: string; params: Record<string, unknown> }) => {
    if (event.toolName !== "exec") return;
```

**委托给 `rtk rewrite`**（`:76-108`）：

```ts
function tryRewrite(command: string): [string | null, RewriteVerdict?] {
  const options = {
    encoding: "utf-8" as const,
    timeout: 2000,
    env: { ...process.env, RTK_REWRITE_HOST },   // ← 关键：声明「审批归 OpenClaw 管」
  };
  try {
    const result = execFileSync("rtk", ["rewrite", command], options).toString().trim();
    // exit 0：无规则命中（或显式 allow）→ 改写并应用
    return [result && result !== command ? result : null];
  } catch (e: any) {
    if (e?.status === 2) return [null, "deny"];                       // exit 2 → deny
    if (e?.status === 3 && e.stdout) {                                // exit 3 → ask
      const result = e.stdout.toString().trim();
      if (result && result !== command) return [result, "ask"];
      return [null];
    }
    return [null];                                                    // exit 1 / 未知 → 透传
  }
}
```

**改写逻辑委托给 `rtk rewrite` 的具体形态**（`:126-200`）：

```ts
const command = event.params?.command;
if (typeof command !== "string") return;

const [rewritten, verdict] = tryRewrite(command);

if (verdict === "deny") {                       // :133-141
  return { block: true, blockReason: "RTK deny rule matched" };
}
if (!rewritten) return;                         // :143  无改写 → 原样返回（undefined）

const result = { params: { ...event.params, command: rewritten } };   // :151-163  ← 就地替换命令

if (verdict === "ask") {                        // :182-198
  result.requireApproval = {
    title: "RTK rewrite suggestion",
    description: `Rewrite: \`${command}\` → \`${rewritten}\``,
    severity: "info",
    timeoutBehavior: "deny",
    allowedDecisions: ["allow-once", "deny"],   // 刻意不含 "allow-always"（OpenClaw 不持久化插件 hook 审批）
    onResolution: (decision: string) => { /* verbose 日志 */ },
  };
}
return result;
```

注册优先级 `{ priority: 10 }`（`:202`）。

**关键设计注释**（`:11-39`）：
- 「RTK owns the deny gate, OpenClaw owns approval.」（`:11`）
- 「All rewrite logic lives in `rtk rewrite` (src/discover/registry.rs). This plugin is a thin delegate — to add or change rules, edit the Rust registry, not this file.」（`:7-9`）
- 宿主名走环境变量而非 argv 的理由（`:33-37`）
- OpenClaw 的 `tools.exec.mode` / `security` / `ask` 会看到**改写后的字符串**（`rtk git push` 而不是 `git push`），所以规则要按 `rtk` 形式写（`:174-181`）

**`openclaw/README.md`** 补充：`:7`（`before_tool_call` 拦截 `exec`）、`:9`（薄委托，RTK 加新 filter 自动生效）、`:64`（`RTK_REWRITE_HOST=openclaw` 的语义）、`:95`（哪些命令不会被改写）

**其它 delegate 对照**（证明这是通用样板）：
- Hermes（Python）：`hooks/hermes/rtk-rewrite/__init__.py:51-92` —— `subprocess.run(["rtk","rewrite",command])`，接受 `{0,3}`（`:14`），**就地改 `args["command"]`**（`:89`），不返回 directive（避免覆盖其他插件的 modify）；超时 5s，`RTK_HERMES_TIMEOUT` 可调（`:43-48`）
- Pi/OMP（TS）：`hooks/pi/rtk.ts:116-136` —— `pi.on("tool_call", ...)`，`event.input.command = rewritten`（`:129`），版本探测 <0.23.0 直接不注册（`:98-113`）
- Claude Code（bash）：`hooks/claude/rtk-rewrite.sh:47-104` —— 读 stdin JSON → `jq -r '.tool_input.command'` → `rtk rewrite` → 按退出码吐 Claude 格式 JSON

---

## 7. hook 完整生命周期与各 agent JSON 格式差异

### 7.1 生命周期（`hooks/README.md:22-35`）

```
Agent runs command (e.g., "cargo test --nocapture")
  -> Hook intercepts (PreToolUse / plugin event)
  -> Reads JSON input, extracts command string
  -> Uses the shared RTK rewrite registry
  -> Registry matches pattern, returns "rtk cargo test --nocapture"
  -> Hook sends response in agent-specific JSON format
  -> Agent executes "rtk cargo test --nocapture" instead
  -> Filtered output reaches LLM
```

`hooks/README.md:35`：「All rewrite logic lives in the `rtk` binary. External hook scripts call `rtk rewrite`; native Rust hook processors call the registry directly.」

### 7.2 `hooks/README.md`（部署产物层） vs `src/hooks/README.md`（生命周期管理层）

| | `hooks/README.md`（根） | `src/hooks/README.md` |
|---|---|---|
| 定位 | **已部署的 hook 产物**（脚本 + agent 配置） | **生命周期管理层**（安装/卸载/完整性/审计/信任） |
| 明确不拥有 | hook 安装逻辑（`src/hooks/init/`）、改写注册表（`src/discover/`）、完整性校验（`src/hooks/integrity.rs`）（`hooks/README.md:9`） | 部署脚本本身（`hooks/`）、改写注册表、命令过滤（`cmds/`）（`src/hooks/README.md:11`） |
| 独有内容 | 各 agent JSON 格式、集成层级表、降级策略 | 安装模式表、完整性五态、权限模型、退出码表、`RTK_REWRITE_HOST` |

`src/hooks/README.md:14` 点明决策唯一入口：

> "`decision.rs` is the single place RTK decides what a hook should do with a command — deny, defer, rewrite-and-allow, or rewrite-and-ask. All three entry points route through it: the in-process `rtk hook <agent>` hosts (`hook_cmd.rs`), the `rtk rewrite` subprocess path (`rewrite_cmd.rs`), and the `rtk hook check` diagnostic."

### 7.3 安装模式（`src/hooks/README.md:23-38`）

| 模式 | 命令 | 产物 |
|---|---|---|
| 默认（全局） | `rtk init -g` | hook + SHA-256 + RTK.md；patch settings.json / CLAUDE.md |
| 仅 hook | `rtk init -g --hook-only` | hook + hash；patch settings.json |
| Claude-MD（旧） | `rtk init --claude-md` | 134 行 RTK 块 |
| Codex | `rtk init --codex` | RTK.md + `.codex/hooks.json`（本地）或 `$CODEX_HOME/hooks.json`（全局）；patch AGENTS.md + `PreToolUse` hook |
| Pi | `rtk init --agent pi` | `.pi/extensions/rtk.ts` |
| OMP | `rtk init --agent omp` | `.omp/extensions/rtk.ts`（与 Pi 同一文件） |
| Hermes | `rtk init --agent hermes` | `~/.hermes/plugins/rtk-rewrite/` + `config.yaml` `plugins.enabled` |
| OpenCode | `rtk init -g --opencode` | OpenCode 插件 |
| Antigravity | `rtk init --agent antigravity` | `plugins/rtk/`（plugin.json / hooks.json / rules/AGENTS.md） |

`AgentTarget` 枚举（`src/lib.rs:40-69`）：Claude / Cursor / Trae / Windsurf / Cline / Kilocode / Antigravity / Kimi / Pi / Hermes / Droid / Vibe / Omp。

### 7.4 完整性校验（`src/hooks/README.md:41-54`）

SHA-256；安装时 `integrity::store_hash()` 写到 `~/.claude/hooks/.rtk-hook.sha256`（只读 0o444）；运行时 `integrity::runtime_check()` 重算比对，被篡改则**阻止执行**。五种状态：Verified / Tampered / NoBaseline / NotInstalled / OrphanedHash。

### 7.5 各 agent JSON 格式差异（`hooks/README.md:73-319`）

| Agent | 机制 | 输入字段 | 输出字段 | 能否改命令 |
|---|---|---|---|---|
| Claude Code | bash hook `PreToolUse` | `tool_name` + `tool_input.command` | `hookSpecificOutput.{hookEventName,permissionDecision,permissionDecisionReason,updatedInput}`（`hooks/README.md:86-97`） | ✅ `updatedInput` |
| Cursor | bash hook | 同 Claude | `{"permission":"allow","updated_input":{"command":...}}`；**无改写必须吐 `{}`**（`hooks/README.md:99-112`） | ✅ `updated_input` |
| Trae | `rtk hook trae` | `tool_name:"RunCommand"` + `tool_input.{command,description}` | 只给 `hookSpecificOutput.updatedInput`，**不给 permissionDecision**（`hooks/README.md:114-138`） | ✅ |
| Codex CLI | `rtk hook codex` | `hook_event_name`,`tool_name`,`permission_mode`,`tool_input` | 同 Claude（`allow` 是 Codex 接受 updatedInput 的必要条件）（`hooks/README.md:140-166`） | ✅ |
| Copilot CLI | `rtk hook copilot` | camelCase：`toolName:"bash"` + `toolArgs`（**JSON 字符串**） | **deny-with-suggestion**：`{"permissionDecision":"deny","permissionDecisionReason":"Token savings: use `rtk git status` instead"}`（`hooks/README.md:168-186`） | ❌ |
| VS Code Copilot Chat | `rtk hook copilot` | snake_case 同 Claude | 同 Claude（含 updatedInput）（`hooks/README.md:188-199`） | ✅ |
| Gemini CLI | `rtk hook gemini` | `tool_name:"run_shell_command"` | `{"decision":"allow","hookSpecificOutput":{"tool_input":{...}}}`；无改写 `{"decision":"allow"}`（`hooks/README.md:201-223`） | ✅ |
| Mistral Vibe | `rtk hook vibe` | `tool_name:"bash"`,`tool_input`,`hook_event_name:"pre_tool"`,`session_id` | `hook_specific_output.tool_input` + `system_message`（`hooks/README.md:225-249`） | ✅ |
| Google Antigravity | `rtk hook antigravity` | `toolCall.{name:"run_command",args:{CommandLine}}` | `{"decision":"allow","reason":...,"overwrite":{"CommandLine":...}}`（`hooks/README.md:251-276`） | ✅ `overwrite.CommandLine` |
| OpenCode | TS 插件 | 内部事件 | 调 `rtk hook opencode` 拿 JSON 的 `.command`（`hooks/README.md:278-305`） | ✅ 就地改 `args.command` |
| Hermes | Python 插件 | 内部事件 | 就地改 `args["command"]`（`hooks/README.md:307-319`） | ✅ |
| Pi / OMP | TS 扩展 | `tool_call` 事件 | 就地改 `event.input.command` | ✅ |
| Cline / Roo Code | 规则文件 | — | `.clinerules` 提示词级 | N/A |
| Windsurf | 规则文件 | — | `.windsurfrules` 提示词级 | N/A |

### 7.6 非阻塞契约（`hooks/README.md:354-370`）

> "Hooks must **never block command execution**. All error paths (missing binary, bad JSON, rewrite failure) must exit 0 so the agent's command runs unmodified."

降级矩阵：jq 缺失 / rtk 缺失 / rtk < 0.23.0 / 非法 JSON / `rtk rewrite` 崩溃 / 过滤器出错 → 全部 exit 0 或回退原始输出。

进程内 hook 也有同一纪律（`src/hooks/README.md:236`）：`hook_cmd.rs` 的处理器**每条路径都必须返回 `Ok(())`**，否则 `Err` 传到 `main()` 会非零退出，反而**阻止**用户命令执行。

### 7.7 集成层级（`hooks/README.md:375-381`）

| 层级 | 机制 | 维护成本 | 例子 |
|---|---|---|---|
| Full hook | shell / Rust 二进制，拦 agent 的 hook API | 高（跟 API 变更） | Claude Code、Cursor、Copilot、Gemini、Codex、Antigravity |
| Plugin | agent 插件系统里的 TS/JS/Python | 中 | OpenCode、Hermes、Pi、OMP |
| Rules file | 提示词级指令 | 低 | Cline、Windsurf |

---

## 8. 可复用性判断（DSH 接入）

### 8.1 DSH 侧已实测的钩子面（本次会话 Cordis Inspect 读取，非文件）

| 事件 | 模式 | 能做什么 | **不能做什么** |
|---|---|---|---|
| `tools/pre-execute` | waterfall | `PreToolDecision = {kind:'allow'} \| {kind:'deny',reason} \| {kind:'cancel'} \| {kind:'ask',reason?,displayReason?}` | ❌ **不能替换 `exec.arguments`**（`ToolExecutionInput.arguments: unknown` 是 readonly） |
| `tools/post-execute` | waterfall | `PostToolDecision = {kind:'accept',content?} \| {kind:'accept',value} \| {kind:'block',feedback}` → **能替换 result content** | — |
| `tools/execute` | waterfall | 包住 dispatch（超时/重试/指标） | — |
| `tools/result` | emit | 观察最终结果 | — |
| `ToolsService.register()` / `setExecutor()` | 服务方法 | **注册/替换一个 ToolDefinition**（含 `execute(args, exec)`），或整体替换 ToolExecutor | — |
| `approval/request` | waterfall | 组合式审批请求 | — |
| `agent/pre-step` | waterfall | 拒绝/替换进入 step 的 messages | — |

**关键结论**：DSH **有** PreToolUse 等价物（`tools/pre-execute`），但它只能「允许/拒绝/取消/询问」，**不能像 Claude Code 的 `updatedInput` 那样替换命令字符串**。所以 rtk 的「改写」语义在 DSH 上**不能**通过 pre-execute 钩子实现。

### 8.2 三个选项的可行性排序

#### (a) 调 `rtk rewrite` 拿改写后的命令 —— **最现实**（但 DSH 侧需要换挂载点）

**支持理由**：
1. 契约极简：**argv 进、stdout 出、退出码承载语义**（`src/hooks/rewrite_cmd.rs:91-125`），无 JSON、无 stdin、无状态。
2. 已有 **4 个** 独立实现的 delegate 证明这个契约可跨语言/跨运行时复用：bash（`hooks/claude/rtk-rewrite.sh:59`）、TypeScript（`openclaw/index.ts:85`、`hooks/pi/rtk.ts:46`）、Python（`hooks/hermes/rtk-rewrite/__init__.py:62-69`）。
3. 失败语义清晰可判：0/1/2/3 四态，每态有确定动作（§2.3）。
4. **纯 Apache-2.0**：这条路径只用 `rtk` CLI 的 rewrite 注册表（`src/discover/registry.rs`）+ 权限判定（`src/hooks/permissions.rs`），**完全不碰 engine**（`src/core/mod.rs:5,14-16,18,22,24,26,31` 用 `#[cfg(feature = "engine")]` 隔离了 pipeline 能力；`src/core/pipeline_config_oss.rs` 是 OSS 版 stub，`src/core/pipeline_config_oss.rs:42` 的 `before_tool_decision` 直接返回 `NoRewrite`）。
5. 幂等：`rtk git status` 原样返回（`src/discover/registry.rs:836-838`）。

**DSH 侧的现实挂载点**（按可行性排序）：

- **A1（最优）**：DSH 插件**替换/包装 terminal 工具**——用 `ToolsService.register()` 注册一个 `rtk` 版 terminal 工具，或用 `ToolsService.setExecutor()` 包一层 Executor，在 `execute(args, exec)` 内部调 `rtk rewrite args.command`，用返回的字符串替换 `args.command` 后再执行。语义等价于 OpenClaw 的 `before_tool_call`（`openclaw/index.ts:151-163`），但落在 DSH 的工具执行面而不是钩子面。
- **A2（次优）**：如果 DSH 允许在 `tools/pre-execute` 之外拦截，用 `tools/execute` waterfall 包住 dispatch，在那里替换参数——但 `tools/execute` 的 `ToolDispatchExecution` 是否可改 arguments 需进一步核实（Inspect 里未展开该类型）。
- **A3**：只有 `tools/pre-execute` 可用时，只能用它做 **deny/ask 闸门**（对应 rtk 的 exit 2/3 语义），拿不到改写后的命令。

#### (b) 把 `rtk <cmd>` 塞进 agent 提示词 —— **能做，但收益最低、最不可控**

- 实现方式：`awareness.level = "full"`（`src/core/config.rs:36-49`；`docs/guide/getting-started/configuration.md:73-77`），`rtk init` 会把「外部程序一律加 rtk 前缀」写进 agent 的指令文件。
- 这是 rtk 自己给「**无 hook 的 agent**」准备的降级路径（`docs/guide/getting-started/configuration.md:86-87`；`hooks/README.md:13-16`）。
- 代价：**完全依赖模型自律**——漏一次前缀就零收益，且模型可能自己造出错误的 rtk 子命令。DSH 侧还多一层：提示词是 system-prompt 拼装的一部分（DSH 有 `system-prompt/assemble` waterfall 可注入），但那属于提示词工程，不是「接入」。
- 唯一优点：零代码、零进程、零失败模式。

#### (c) 用 pipeline TOML 自己实现过滤 —— **在本仓库里是「部分可行」，但对 DSH 不现实**

- **可行部分**：`filters.toml`（TOML filter DSL）能纯声明「某命令 → 行级剥离规则」，Apache-2.0 就够（`src/filters/README.md:29-62`）。但它要求命令**由 rtk 执行**（`rtk <tool> args`），DSH 的工具不会自动走 rtk。
- **不可行部分**：`pipeline.toml` 的 `hook.after_tool.*` 压缩链需要 `rtk-engine`（**BUSL-1.1**）+ 引擎侧的 after_tool 宿主（`src/hooks/posthook.rs:1-18` 是 Claude 协议适配器，`rtk hook` 系列目前只有 before_tool 语义）。
- **另有一条更干净的 DSH 侧路径**：`rtk pipe --filter <name>` 是**纯 stdin→stdout** 接口（`src/lib.rs:850-859`；`src/cmds/system/pipe_cmd.rs:12-41` 列出 20+ 个过滤器名：`cargo-test`/`pytest`/`go-test`/`tsc`/`vitest`/`grep`/`find`/`git-log`/`git-diff`/`git-status`/`log`/`mypy`/`ruff-check`/`ruff-format`/`sqlfluff-lint`/`prettier`/`phpunit`/`pest`/`ecs`/`phpstan`/`pint`…）。**这正好能挂 DSH 的 `tools/post-execute`**——把 tool result 的 output 喂进 `rtk pipe --filter git-status`，拿回压缩文本替换 content。
  - 失败模式安全：过滤失败回退原始输出（`src/cmds/README.md:250`）；`guard::never_worse` 保证不比原文更大（`src/core/runner.rs:14-21`；`src/core/guard.rs`）
  - 但注意：`rtk pipe` 走的是**固定过滤器名**（Rust 里的纯函数），不是用户可写的任意规则；要用户自定义仍需 `filters.toml` + `rtk <tool>` 执行路径。

### 8.3 综合建议（给 DSH 侧）

| 目标 | 推荐接入面 | 需要的 DSH 能力 | rtk 侧调用 | 许可 |
|---|---|---|---|---|
| 命令改写（`git status`→`rtk git status`） | **A1：包装 terminal 工具 / Executor** | `ToolsService.register` 或 `setExecutor` | `rtk rewrite <cmd>` → exit 0/3 用 stdout；exit 1 透传；exit 2 拒绝 | Apache-2.0 |
| 权限闸门（deny/ask） | `tools/pre-execute` | waterfall 决策 | 同上，读 exit 2/3 | Apache-2.0 |
| 输出压缩（PostToolUse 等价） | `tools/post-execute` | `accept.content` 替换 | `rtk pipe --filter <name>`（stdin→stdout） | Apache-2.0 |
| 声明式自定义过滤 | `filters.toml` + `rtk <tool>` | 需 rtk 执行命令 | n/a | Apache-2.0 |
| 声明式压缩链 / 任务委派 | `pipeline.toml` | 需 engine + after_tool 宿主 | n/a | **BUSL-1.1** |

**一句话**：DSH 侧最现实的是 **(a)**，但要把「改写」从 `tools/pre-execute` 挪到**工具包装/Executor 层**；同时 **(c) 的 `filters.toml` 部分**和 **`rtk pipe`** 可以补齐输出侧。

### 8.4 还需要核实/未闭环的点

- DSH 的 `tools/execute` waterfall 的 `ToolDispatchExecution` 是否允许替换 arguments（Inspect 未展开该类型声明）——这决定 A2 是否可行
- DSH 的 terminal 工具名与参数结构（`TerminalCallView` 提示是 `card:'terminal'`，但具体工具名/入参 schema 需查 `listTools`）
- rtk 的 `rtk hook opencode` 输出 JSON 的完整字段（已知 `.command`，`hooks/README.md:294`）
- rtk 的 `rtk` 二进制在 Windows 上的可用性（`Cargo.toml:95-100` 有 windows-sys 依赖，但 `hooks/claude/rtk-rewrite.sh` 等 bash 脚本在 Windows 不可用；`src/hooks/init/` 有 Windows 分支）
- **文档与实现的一处出入**：`docs/usage/FEATURES.md:1305` 称 `rtk rewrite` 的 exit 2「With output, an rtk pipeline rule denied or routed the command」，但代码路径显示 `rtk rewrite` **不查 pipeline**——`src/hooks/rewrite_cmd.rs:103-124` 只走 `permissions::check_command` + `decision::decide`，而 pipeline 的 `before_tool.shell` 求值只发生在 Claude 的进程内 hook（`src/hooks/hook_cmd/claude_pipeline.rs:141-145`，调用 `pipeline_config::before_shell_decision`，`src/core/pipeline_config.rs:534-547`）。`src/hooks/permissions.rs` 中**零** pipeline 引用（grep 无匹配）。**DSH 集成时不要依赖 `rtk rewrite` 的 pipeline 语义。**

---

## 9. 许可与风险

### 9.1 边界

| 组件 | 许可 | 证据 |
|---|---|---|
| `rtk` CLI（`src/`, `text/`） | **Apache-2.0** | `Cargo.toml:30`；根 `LICENSE`（Apache 2.0 全文）；`text/Cargo.toml:9` |
| `rtk-engine`（`engine/`） | **BUSL-1.1** | `engine/Cargo.toml:8`；`engine/LICENSE:1` |
| `rtk-plugin`（`plugin/`） | **BUSL-1.1** | `plugin/Cargo.toml:8` |
| `rtk-proxy`（`proxy/`） | **BUSL-1.1** | `proxy/LICENSE:1`；`proxy/README.md:9` |
| `rtk-text`（`text/`） | Apache-2.0 | `text/Cargo.toml:9` |

`Cargo.toml:1-4` 的设计说明：

> "Two crates, one binary. `rtk` (Apache-2.0) is the CLI; `engine` (BUSL-1.1) is `rtk-engine`, the pure compression core the CLI LINKS -- so the shipped `rtk` binary contains BUSL-1.1 code, and engine/LICENSE carries an Additional Use Grant covering production use as part of that binary."

**关键**：默认构建 `default = ["engine"]`（`Cargo.toml:47`）→ **官方发行的 `rtk` 二进制里含 BUSL 代码**。纯 Apache 版是 `cargo build --release --no-default-features`（`Cargo.toml:68`；`docs/guide/resources/editions.md:64-68`），此时 `rtk-engine` 完全不在依赖图里。

### 9.2 BUSL-1.1 的 Additional Use Grant 原文（`engine/LICENSE:15-25`）

```
Additional Use Grant: You may use the Licensed Work in production, without
                      charge, including as a component of the `rtk`
                      command-line binary and the `rtk-proxy` daemon
                      distributed by RTK AI Labs. This grant does not permit
                      offering the Licensed Work, or a derivative of it, to
                      third parties as a paid or hosted product or service;
                      for that, contact contact@rtk-ai.app.

Change Date:          2030-09-09
Change License:       Apache License, Version 2.0
```

### 9.3 商业使用影响（对 DSH 场景的具体判断）

| 集成方式 | 许可暴露 | 判断 |
|---|---|---|
| **DSH 插件 spawn `rtk rewrite` 子进程**（用 Apache 版 `rtk-oss` 或任意 Apache-2.0 构建） | 无 | ✅ **安全**。子进程调用是「使用」不是「链接/衍生」；Apache-2.0 只要求保留 NOTICE/许可证文本 |
| **DSH 插件 spawn 官方 `rtk` 二进制**（含 engine） | 间接 | ⚠️ 若把该二进制**随 DSH 一起分发**给第三方 → 触发 engine 的 Additional Use Grant 限制（`engine/LICENSE:18-21`）；若只是用户自己装，则「免费生产使用」被明确允许（`engine/LICENSE:15-17`） |
| **DSH 链接 `rtk-engine` / `rtk-plugin` crate** | **BUSL-1.1 传染** | ❌ 衍生作品需商业授权，直到 2030-09-09 |
| **DSH 起 `rtk-proxy` 守护进程** | BUSL-1.1 | ⚠️ 代理本身作为「付费/托管产品的一部分」再分发 → 受限 |

### 9.4 两个发行版（`docs/guide/resources/editions.md:12-18`）

| 下载物 | 二进制 | 许可 | 内容 |
|---|---|---|---|
| `rtk-<target>` | `rtk` | Apache-2.0 **+** BUSL-1.1 | hook 核心 + 全部命令过滤器 **+ 压缩引擎** |
| `rtk-oss-<target>` | `rtk` | **仅 Apache-2.0** | hook 核心 + 全部命令过滤器，**无引擎** |
| `rtk-proxy-<target>` | `rtk-proxy` | BUSL-1.1 | 代理守护进程，两个 `rtk` 版都不捆绑 |

Apache-2.0 版失去什么（`docs/guide/resources/editions.md:32-38`）：`rtk pipeline`、`rtk methods`、`rtk tasks`、**PostToolUse 压缩**、pipeline 文件全部失效。

**保住的**（`docs/guide/resources/editions.md:40`）：

> "**What stays is the part you use all day.** The PreToolUse rewrite hook — the mechanism that turns your agent's `git status` into `rtk git status` — is Apache-2.0 and behaves identically. So does every command filter, `rtk init`, `rtk gain`, `rtk discover`, `rtk session`, `rtk recall`, and `rtk llm-proxy`."

两个版**同名同路径**（`/usr/bin/rtk`），**不能共存**（`docs/guide/resources/editions.md:18, 60`）。

### 9.5 风险清单

1. **BUSL 传染（高）**：任何 `cargo add rtk-engine` / `rtk-plugin` 的做法，直接把 DSH 变成 BUSL 衍生作品。**规避**：只用子进程调用 + Apache-2.0 版二进制。
2. **官方 `rtk` 二进制含 BUSL（中高）**：随产品分发的风险。**规避**：用 `rtk-oss`（`RTK_EDITION=oss sh install.sh`，`docs/guide/resources/editions.md:48-50`）或 `--no-default-features` 自建。
3. **pipeline 能力在 Apache 版缺席（中）**：如果 DSH 想用 pipeline.toml 做声明式压缩链，只能在非商业场景用，或接受 BUSL。
4. **`rtk-proxy` 协议稳定性（低-中）**：`proxy/README.md:19-21` 明说「Today Claude Code is wired... other agents follow as their redirection is verified end to end」——**非 Claude agent 的重定向尚未端到端验证**。DSH 要用代理模式需要自己验证。
5. **上游兼容性（中）**：`rtk rewrite` 需 ≥ 0.23.0（`hooks/claude/rtk-rewrite.sh:26-45`），当前版本 `0.49.0`（`Cargo.toml:25`）。接口契约由 `tests/hook_decision_protocol_test.rs` pin 住（`src/hooks/decision.rs:83, 416`），说明团队重视兼容性——正面信号。
6. **平台（中）**：多数 hook 产物是 bash / Python / TS；Windows 上 bash hook 不可用，`src/hooks/init/` 有 Windows 分支但需单独验证。DSH 是跨平台的。
7. **安全边界（中）**：rtk 自己有信任门控机制（filter 文件信任 `src/filters/README.md:124-141`；pipeline 项目文件信任 `docs/guide/pipelines/writing-pipelines.md:19`），并且明确自陈「这是同意 + 防篡改，不是沙箱」（`src/filters/README.md:142`）。DSH 侧若引入 rtk 作为子进程，等于把「agent 实际执行的命令」交由一个外部进程决定改写——需要接受 rtk 的 deny 门控作为唯一授权来源。

---

## 附：关键文件速查

| 主题 | 文件:行 |
|---|---|
| CLI 入口 / `Commands` 枚举 | `src/lib.rs:86-104`, `src/lib.rs:1108` |
| `rtk rewrite` CLI 定义 | `src/lib.rs:1067-1101` |
| `rtk rewrite` 分发 | `src/lib.rs:3726-3730` |
| `rtk rewrite` 实现 | `src/hooks/rewrite_cmd.rs:91-125` |
| 退出码协议表 | `src/hooks/rewrite_cmd.rs:70-77`；`src/lib.rs:1067-1095`；`docs/usage/FEATURES.md:1299-1314` |
| 决策核心 | `src/hooks/decision.rs:156-260` |
| `RTK_REWRITE_HOST` | `src/hooks/decision.rs:71-153` |
| 改写注册表 | `src/discover/registry.rs:768-820` |
| 改写规则定义 | `src/discover/rules.rs`（说明见 `src/discover/README.md:111-119`） |
| 执行骨架 | `src/core/runner.rs:574-702` |
| FilterMode | `src/cmds/README.md:65-72` |
| 配置结构 | `src/core/config.rs:10-34`, `docs/guide/getting-started/configuration.md:24-64` |
| `[hooks]` | `src/core/config.rs:118-166` |
| `[awareness]` | `src/core/config.rs:36-71` |
| pipeline 语法 | `docs/guide/pipelines/writing-pipelines.md:35-50, 123-166` |
| 内置 pipeline 实体 | `engine/builtin/*.toml` |
| TOML filter DSL | `src/filters/README.md:29-62` |
| 信任机制 | `src/filters/README.md:124-141` |
| engine 定位 | `engine/README.md:3` |
| proxy 定位 | `proxy/README.md:14-33` |
| rtk-plugin SDK | `plugin/src/lib.rs:1-33` |
| OpenClaw 插件 | `openclaw/index.ts:76-108`（委托）, `:122-203`（hook） |
| Claude bash hook | `hooks/claude/rtk-rewrite.sh:47-104` |
| Hermes Python 插件 | `hooks/hermes/rtk-rewrite/__init__.py:51-92` |
| Pi/OMP TS 扩展 | `hooks/pi/rtk.ts:116-136` |
| 各 agent JSON 格式 | `hooks/README.md:73-319` |
| 非阻塞契约 | `hooks/README.md:354-370`；`src/hooks/README.md:234-236` |
| `rtk pipe` | `src/lib.rs:850-859`；`src/cmds/system/pipe_cmd.rs:12-41` |
| PostToolUse 适配器 | `src/hooks/posthook.rs:1-18` |
| 许可边界 | `Cargo.toml:1-19, 30, 44-49`；`engine/LICENSE:14-25`；`docs/guide/resources/editions.md:12-18, 32-40` |
