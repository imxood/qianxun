# rtk 接入 DSH 的可行性评估与落地方案

> 评估对象：外部 Rust 库 `E:\git\ai\rtk`（rtk / Rust Token Killer）
> 评估目标：千寻（DSH 宿主）是否、以及如何接入 rtk
> 日期：2026-11-11
> 底稿：
> - `docs/analysis/17-1-rtk内部机制与DSH接入分析.md`（rtk 侧机制与许可）
> - `docs/22-DSH运行时工具拦截能力探查报告.md`（DSH 侧拦截面）
> 本文只做**综合与决策**，不重复底稿细节。

---

## 0. 一句话结论

**rtk 值得接，但不要用它的「hook 改写命令」主路线接。** DSH 官方明确禁止在工具事件里改写入参
（`dsh-tools/README.md:197`），rtk 的核心机制在 DSH 上**没有原生挂载点**；真正顺的接法是
**输出侧压缩**：`tools/post-execute` + `rtk pipe --filter <name>`（stdin→stdout，Apache-2.0），
不动命令、不改权限语义、不增加进程层级。

---

## 1. rtk 是什么

### 1.1 一句话

一个 Rust 单文件 CLI **代理**：在命令输出进入 LLM 上下文**之前**做过滤与压缩，把
`git status` / `cargo test` / `vitest` / `tsc` 这类命令的原始输出压掉大部分字节。

### 1.2 工作原理（实测确认）

```
agent 发 "cargo test"
  → （可选）hook 把命令改写成 "rtk cargo test"
  → rtk spawn 真正的 cargo，捕获 stdout/stderr
  → 按命令类型用专用过滤器压缩（失败聚焦 / 按规则分组 / 状态机解析 / NDJSON 流式…）
  → 打印压缩结果 + 一行 `[full output: rtk recall <hash>]` 提示（可回捞原文）
  → 记账进 SQLite（`~/.local/share/rtk/history.db`，90 天窗口）
```

- 代理骨架：`src/core/runner.rs:574-702`（run_filtered / run_streamed / run_passthrough）
- 12 种过滤策略分类：`docs/contributing/ARCHITECTURE.md:206-306`
- 失败即回退原始输出，且保证不会比原文更大（`src/core/guard.rs`；`src/cmds/README.md:250`）

### 1.3 它**不是**什么

| 误解 | 事实 |
|---|---|
| 是 agent 框架 / 编排器 | 否。它只是**输出层的文本压缩器**，规则全在 Rust 侧 |
| 能省 90% 账单 | 否。90% 指的是 **bash 输出字节数**；账单还含提示词、对话历史、输出 token（`README_zh.md:59-63`） |
| 是个 HTTP 代理 | `proxy/` 才是（拦截 LLM provider 流量），且**非 Claude agent 的重定向尚未端到端验证**（`proxy/README.md:19-21`） |

### 1.4 三层集成面（它自己定义的）

| 层 | 机制 | 覆盖的 agent |
|---|---|---|
| Hook | `PreToolUse` 等价物改写命令 | Claude Code / Cursor / Codex / Trae / Gemini / Copilot / Droid / Antigravity / Vibe |
| Plugin | agent 插件里的 `before_tool_call` 类钩子 | OpenCode / OpenClaw / Pi / OMP / Hermes |
| Rules | 只写提示词，靠模型自律 | Cline / Windsurf / Kilo Code |

**DSH 不在这张表里**——没有 `PreToolUse`、没有 `updatedInput`（330 个 DSH 包 grep `PreToolUse|updatedInput` = 0 命中）。

---

## 2. DSH 侧：能接什么，不能接什么

### 2.1 硬约束（已实测复核）

| 事件 | 能做 | 不能做 |
|---|---|---|
| `tools/pre-execute` | allow / deny / cancel / ask | ❌ **不能替换 `exec.arguments`**（`dsh-tools/lib/types/contracts.d.ts:318`：「Input rewriting is excluded because arguments are already logged and presented.」；`dsh-tools/README.md:197` 称改写设计只是**拟议中的 Agent Note**） |
| `tools/post-execute` | ✅ **可替换 result content**（`dsh-tools/lib/types/events.d.ts:40`） | — |
| `tools/execute` | 只能换 signal | 不能改参数 |
| `ctx.shell` 执行器 | ✅ **可在 `resolve()` 里改写 `command`** | — |

> 复核记录：`dsh-tools/lib/types/contracts.d.ts:318` 与 `README.md:197` 原文已由 Lead 亲自 grep 确认；
> `dsh-pwsh-local/lib/index.js:173-179` 的 `resolve(request)` 返回 `command: request.command` 已确认。

### 2.2 于是，三条可行路线

| 路线 | 机制 | 收益 | 代价 | 许可 |
|---|---|---|---|---|
| **R1 输出侧压缩**（推荐） | `tools/post-execute` 把结果喂给 `rtk pipe --filter <name>`，用返回文本替换 content | 中～高（正是 rtk 的主战场：测试/编译/lint 输出） | 需在插件里识别「这条命令属于哪个过滤器」；不省执行侧输出 | Apache-2.0 |
| **R2 执行器改写** | 继承 `ctx.shell` 的执行器（`dsh-pwsh-sandbox/lib/index.js:118` 就是子类范例），在 `resolve()` 里把 `git status` 改成 `rtk git status` | 高（连 stdout 前的噪音也省） | 多一层进程（rtk→真命令）；**改写后的命令不再经过 DSH 的任何内容级校验**；DSH 日志与实际执行内容不一致 | Apache-2.0 |
| **R3 提示词引导** | system prompt 告知模型「外部命令加 `rtk ` 前缀」 | 低 | 全靠模型自律，漏一次零收益 | Apache-2.0 |

R2 还有一个已知坑：**同一 scope 层不能重名注册工具**——`pwsh` 已被 agent preset 占用该 scope，
同层重名直接抛 `tool "pwsh" is already registered in this scope`（`dsh-tools/lib/index.js:385`），
可行的变体是「把 preset 的 `tool-pwsh` 行换成自研包」。

### 2.3 为什么 R1 排在 R2 前面

1. **不碰命令** ⇒ 不碰审批、不碰沙箱、不碰 DSH 的命令审计日志；
   DSH 现状是**没有命令内容级策略**（`validateBashArgs` 只查非空，`dsh-tool-bash/lib/index.js:235-242`），
   一旦引入改写，就等于把「agent 实际跑什么」的决定权交给一个外部进程。
2. **不多一层进程**：R2 是 `rtk → cargo → cargo`（两次 spawn + 5~15ms 开销，`ARCHITECTURE.md:34`）；
   R1 是一次 post-execute 里的 `rtk pipe` 子进程。
3. **失败安全**：`rtk pipe` 过滤失败回退原文，且 `never_worse` 保证不比原文大
   （`src/core/runner.rs:14-21`；`src/cmds/system/pipe_cmd.rs:4`）。
4. **许可干净**：`src/cmds/system/pipe_cmd.rs` 全文无 `cfg(feature = "engine")`，
   过滤器全部来自 Apache-2.0 的命令模块 ⇒ **Apache 版（`--no-default-features`）即可用**（已实测 grep 确认）。

### 2.4 R1 的过滤器覆盖面（`src/cmds/system/pipe_cmd.rs:12-41`）

`cargo-test` / `pytest` / `go-test` / `go-build` / `ctest` / `tsc` / `vitest` / `grep` / `rg` /
`find` / `fd` / `git-log` / `git-diff` / `git-status` / `log` / `mypy` / `ruff-check` / `ruff-format` /
`sqlfluff-lint` / `prettier` / `phpunit` / `pest` / `paratest` / `ecs` / `phpstan` / `pint`

对照千寻工具链（`package.json:12-31`）：

| 千寻常用命令 | rtk 覆盖 | 备注 |
|---|---|---|
| `vitest run` | ✅ `vitest` | |
| `cargo test/clippy/fmt` | ✅ `cargo-test` | clippy 走 `rtk cargo clippy` |
| `git status/diff/log` | ✅ `git-status` / `git-diff` / `git-log` | |
| `prettier --check .` | ✅ `prettier` | |
| `tsc` / `svelte-check` | ⚠️ 仅 `tsc`；`svelte-check` **无覆盖** | |
| `eslint .` | ⚠️ 有 `lint_cmd.rs` 但**不在 pipe 过滤器表里** | |
| `playwright test` | ⚠️ 有 `playwright_cmd.rs`，但不在 pipe 表 | |
| `pnpm dev/build` | ❌ 无 | |

> 覆盖率结论：**测试/编译/git 三类（也是输出最肥的三类）全覆盖**；lint 类覆盖不全。

---

## 3. 对千寻的实际价值评估

### 3.1 收益从哪来

千寻的 agent 在**用户真实项目**里跑命令，输出直接进上下文；`vitest`/`cargo`/`git` 的原始输出动辄数千行。
rtk 压缩的是这一段，收益真实存在。

### 3.2 但要打折的三点

1. **DSH 已有输出裁剪机制**：`dsh-output-retention`（纯截断库）、`dsh-compaction-tool-result-pruner`、
   `dsh-spill-policy`（`dsh-spill-policy/lib/index.js:237` 是 post-execute 改结果的现成范例）。
   rtk 与它们的**边际收益差**未实测——rtk 的独特价值是「按命令类型语义化压缩」（只显示失败、按规则分组），
   而 DSH 现有的是「按大小截断」。语义化 > 截断，但必须实测。
2. **rtk 的收益宣称只覆盖 bash 输出字节**（`README_zh.md:59-63`），不含提示词/历史/输出 token。
3. **千寻自身已有命令纪律**（`AGENTS.md` 全局规则：命令调用只用 Python、here-string 管道等），
   本身已经压掉了很大一部分噪音输出。

### 3.3 风险清单（含缓解）

| # | 风险 | 证据 | 缓解 |
|---|---|---|---|
| R1 | **过滤器有硬上限**：错误最多 20 条、警告 10 条、列表 20 条——agent 可能只看到前 20 个错误就以为修完了 | `src/core/truncate.rs:5-11`；`src/cmds/js/lint_cmd.rs:504`（`MAX_ISSUES = CAP_ERRORS`） | 落地时**必须**保留 `[full output: rtk recall <hash>]` 提示并在插件里显式告知模型可回捞；或对「错误类」命令放弃压缩 |
| R2 | **BUSL 传染**：`cargo add rtk-engine` / `rtk-plugin` 会把接入方变成 BUSL 衍生作品 | `engine/Cargo.toml:8`；`plugin/Cargo.toml:8`；`Cargo.toml:1-4` | 只**子进程调用** CLI，绝不链接 crate；分发用 `rtk-oss` / `--no-default-features` 构建 |
| R3 | **官方 `rtk` 二进制默认含 BUSL 代码**（`default = ["engine"]`，`Cargo.toml:47`） | `Cargo.toml:47` | 随千寻分发时改用 `rtk-oss`（`docs/guide/resources/editions.md:48-50`） |
| R4 | `cat`/`head`/`tail` 会被改写成 `rtk read` | `src/discover/registry.rs:1399-1434` | **好消息**：`rtk read` 默认 `--level none`（`src/lib.rs:202-204`）= 全文，不丢函数体；风险只在用户主动用 `--level aggressive` |
| R5 | 单文件读取有 10 MiB 上限（`RAW_CAP`，`src/core/stream.rs:423`） | 同左 | 千寻场景一般不触发；仍需在超限时回退原文 |
| R6 | **Windows 适配未验证**：多数 hook 产物是 bash/Python/TS | `src/hooks/init/` 有 Windows 分支但需实测；本机**当前未安装 rtk** | 落地前先在 Windows 上跑通 `rtk rewrite` 与 `rtk pipe` |
| R7 | rtk 文档与实现有两处出入：`FEATURES.md:1305` 称 `rtk rewrite` 的 exit 2 会带 pipeline 拒绝消息——**代码不成立**（`src/hooks/rewrite_cmd.rs:103-124` 只走 permissions + decision，不查 pipeline） | 同左 | 集成时**不要依赖 rewrite 的 pipeline 语义** |

---

## 4. 落地方案（分三阶段）

### P0 先量收益（0.5 天，零代码）

1. 装 **rtk-oss**（Apache-2.0）：`RTK_EDITION=oss` 安装脚本，或 `cargo install --no-default-features`。
2. 在千寻仓库跑对照：`git status` / `cargo test` / `pnpm check` / `vitest run` 各跑一次原始版与 rtk 版，
   比字节数；并用 `rtk gain` 记账 30 天。
3. **同时**测 DSH 现有裁剪（output-retention / pruner）的效果——如果 DSH 已经压得差不多，**整件事到此为止**。
4. 产出：一张「命令 → 原始字节 → rtk 后字节 → DSH 现有裁剪后字节」的对照表。
   决策门：**若 rtk 相对 DSH 现状的额外收益 <20%，停止**，不进入 P1。

### P1 提示词级（0.5 天）

在 DSH 的 system prompt section 里告知模型 rtk 约定（对应 rtk 的 `awareness.level = "full"` 路线）。
**零代码、可随时回滚**，先让模型自己习惯 `rtk git status`。
同时在千寻自己的 `AGENTS.md` 里加一条「长输出命令优先加 `rtk ` 前缀」。

### P2 插件级（2~3 天）

在 DSH profile 里落一个自研插件（形态照抄 `src-tauri/src/bridge/assets/index.js` 的 `qx-bridge`：
零依赖纯 ESM + `cordis.patch.yml` 插入），内容：

1. **输出侧压缩（先做这个）**：订阅 `tools/post-execute`
   - 从 `exec.arguments.command` 判定命令族（git / cargo / vitest / tsc / prettier / pytest …）
   - 命中则 `spawn('rtk', ['pipe', '--filter', <name>])`，把 result content 喂 stdin，拿回压缩文本
   - **只替换「压缩后更短」的结果**（`never_worse` 语义），否则保留原文
   - 保留 rtk 的 `[full output: rtk recall <hash>]` 提示
2. **（可选）执行器改写**：后续再评估 R2，先确认 P0 的对照表值得
3. **工具识别**：`dsh-tool-fs-search/lib/index.js:918-933` 是现成的 post-execute 改结果范例，直接抄

### 不建议现在做

- ❌ 把 `rtk-engine` / `rtk-plugin` 编进任何 Rust crate（BUSL 传染）
- ❌ 依赖 `rtk rewrite` 的 pipeline 语义（实现与文档不符，R7）
- ❌ 依赖 `rtk-proxy` 做 LLM 流量拦截（对非 Claude agent 尚未验证）
- ❌ 用 `pipeline.toml` 做声明式压缩链（需要 engine + after_tool 宿主，且 engine 是 BUSL）

---

## 5. 未闭环项（需下一步实测）

1. **DSH `tools/post-execute` 里 result content 的确切形态**——是单个字符串，还是 stdout/stderr 分离的结构化对象？
   这决定 R1 是「整段喂 stdin」还是「按流分别喂」。需要查 `dsh-tools/lib/types/contracts.d.ts` 的
   `ToolExecutionResult` 定义，或在真机跑一次 post-execute 探针。
2. **`rtk pipe` 在 Windows 上的行为**（stdin 传输、编码、换行）——本机未安装 rtk，未实测。
3. **rtk 二进制随千寻分发的路径**——`rtk-oss` 的获取渠道与体积，以及「用户没装 rtk 时插件如何降级」
   （应直接跳过压缩，不报错）。
4. **`dsh-terminal-bash` / `dsh-experimental-worktree` 等绕过 `ctx.shell` 的路径**是否也需要覆盖
   （若走 R2 路线则必须覆盖；走 R1 则不受影响）。
5. **千寻现有 `dsh-output-retention` / pruner 的实际裁剪力度**——P0 的核心对照项。

---

## 6. 附：证据速查

| 结论 | 证据 |
|---|---|
| rtk 是输出压缩代理 | `src/core/runner.rs:574-702`；`src/cmds/README.md:5` |
| 12 类过滤策略 | `docs/contributing/ARCHITECTURE.md:206-306` |
| 失败回退 + never_worse | `src/cmds/README.md:250`；`src/core/guard.rs` |
| `rtk rewrite` 契约（argv 进/stdout 出/退出码 0-3） | `src/hooks/rewrite_cmd.rs:91-125`；`src/hooks/decision.rs:83-153` |
| OpenClaw 插件样板（最接近的 agent 插件接入） | `openclaw/index.ts:122-203` |
| DSH 禁止 pre-execute 改写入参 | `dsh-tools/lib/types/contracts.d.ts:318`；`dsh-tools/README.md:197` |
| DSH post-execute 可替换 result content | `dsh-tools/lib/types/events.d.ts:40`；范例 `dsh-tool-fs-search/lib/index.js:918-933` |
| shell 执行器可子类化改写 command | `dsh-pwsh-local/lib/index.js:173-179`；`dsh-pwsh-sandbox/lib/index.js:118`；`dsh-bash-local/lib/index.js:8-12` |
| 同名工具不可同 scope 注册 | `dsh-tools/lib/index.js:385` |
| `rtk pipe` 过滤器清单 & 无 engine 依赖 | `src/cmds/system/pipe_cmd.rs:12-41`（全文无 `cfg(feature="engine")`） |
| 截断上限 20/10/20 | `src/core/truncate.rs:5-11` |
| `rtk read` 默认全文 | `src/lib.rs:202-204` |
| 许可边界 | `Cargo.toml:30,44-48`；`engine/Cargo.toml:8`；`engine/LICENSE:15-25`；`docs/guide/resources/editions.md:12-68` |
