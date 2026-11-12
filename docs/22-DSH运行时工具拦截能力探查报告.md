# DSH 运行时工具拦截能力探查报告

> 目的：判断 DeepSeek Harness (DSH) 是否具备「拦截 / 改写 agent 工具调用」的能力，
> 以便决定 rtk 这类「命令重写代理」能否、以及以何种方式接入。
>
> 探查对象：DSH 实现 checkout
> `C:\Users\maxu\.qianxun\dsh-runtime\node_modules\.pnpm\@deepseek-ai+dsh-harness-de_c09760371c068422439dcd31c360e712\`
> （该 checkout 只是 `@deepseek-ai/dsh-harness-dev-context` / `dsh-agent` / `dsh-session` 等几个包的挂载点；
> **真正的实现分布在 `C:\Users\maxu\.qianxun\dsh-runtime\node_modules\.pnpm\@deepseek-ai+*` 共 330 个包里**，
> 下文所有 `文件:行号` 均以这些包的 `lib/` 下源码为准。）
>
> 探查时间：2026-11-11　｜　只读，未修改 DSH checkout 任何文件。

---

## 0. 一句话结论

**DSH 明确不支持「在工具调用事件里改写入参」**（官方把它列为「尚未实现的设计提案」），**但存在一条完全可用的替代路径：在 `ctx.shell` 执行器层改写 `command`**——
即写一个继承 `PwshLocalExecutor` / `SandboxPwshExecutor`（或 `LocalBashExecutor`）的子类，
在 `resolve(request)` 返回前把 `spec.command` 改写成 `rtk <cmd>`，
再在 composition 里把 `pwsh-sandbox` / `bash-sandbox` 这一行换成本插件。
所有 shell 类工具（`pwsh` / `bash` / persistent 变体）都经过 `ctx.shell`，因此**一次拦截覆盖全部**。

- 选项 (a)「插件在工具执行前改写命令字符串」→ **成立，但载体不是事件钩子，而是 shell 执行器插件**（详见 §7 路线 A）
- 选项 (b)「只能在提示词层引导」→ 成立但最弱（§7 路线 D）
- 选项 (c)「只能包一层 shell 包装器」→ 成立（§7 路线 C，经典 PATH shim 形态）
- 选项 (d)「完全不支持」→ **不成立**

同时：**输出压缩 / token 统计这类「结果侧」能力，DSH 有一等公民的扩展点**
（`tools/post-execute` waterfall + `ctx.tools.guard()`），可以直接用事件实现，无需改写工具。

---

## 1. DSH 架构

### 1.1 底座：Cordis 插件体系

DSH 0.2.2-alpha.3 建立在 `@deepseek-ai/cordis@4.0.6-alpha.2` 上。所有能力都是 Cordis plugin：
- 插件导出 `export function apply(ctx, config) {}`，可选 `export const inject = [...]` / `export const Config`
  （规范：`dsh-agent-preset/skills/cordis-plugin-development/references/host-plugin.md:57-64`）
- Service 通过 `class X extends Service { constructor(ctx){ super(ctx, "<name>") } }` 注册
  （例：`dsh-shell/lib/index.js:83-85` → `super(ctx, "shell")`）

### 1.2 host / client 两侧

| 侧 | 关键包 | 职责 |
|---|---|---|
| **Host**（Node 进程，agent 真身在此） | `dsh-app-boot`（profile 引导）、`dsh-cordis-host-runner`（`node:vm` realm 内的动态包 + inspect registry）、`dsh-host-webserver`、`dsh-tools`、`dsh-agent-loop`、`dsh-session`、`dsh-llm`、`dsh-shell` + `dsh-pwsh-local`/`dsh-pwsh-sandbox`、`dsh-plugin-manager`、`dsh-agent-preset-registry` | 工具执行、模型调用、会话日志、权限、插件装配 |
| **Client**（浏览器） | `dsh-cordis-client-runner`（浏览器 realm）、`dsh-web-app` / `dsh-web-frontend`、`dsh-client-connection`、`dsh-client-modules`、约 50 个 `dsh-client-ui-*` | GUI 渲染、设置页、审批 UI |

两侧通过 `dsh-typert-registry` / `dsh-api-gateway`（typert）/ `dsh-sdk-jsonrpc-server` 通信。
`dsh-cordis-host-runner/README.md:8` 明确："Host halves run in a `node:vm` realm; browser halves use the Client runner and approval UI."

### 1.3 核心 Service 名（实测枚举，`super(ctx,"<name>")`）

工具与执行相关（rtk 关心的部分）：

| Service 名 | 提供者包 | 说明 |
|---|---|---|
| `tools` | `dsh-tools` | 工具注册表 + 作用域化派发器 |
| `shell` | `dsh-shell`（抽象）→ `dsh-pwsh-local`/`dsh-bash-local`（本地）、`dsh-pwsh-sandbox`/`dsh-bash-sandbox`（受限） | **shell 执行缝隙**，`ctx.shell.resolve()` / `ctx.shell.execute()` |
| `subprocess` | `dsh-subprocess` | 进程 spawn 底层 |
| `jobs` | `dsh-jobs` | 后台任务注册表 |
| `systemPrompt` | `dsh-system-prompt` | 提示词分段（插件可注册 section） |
| `workingDirectory` / `shellEnv` | `dsh-working-directory` / `dsh-shell-env` | cwd 解析 / `DSH_*` 环境 |
| `sandbox` / `sandboxPolicy` | `dsh-sandbox` / `dsh-sandbox-policy` | 沙箱模式与策略 |
| `approval` | `dsh-user-approval` | 审批问答（fail-closed） |
| `permissionPresets` | `dsh-permission-presets` | 预设（sandbox + approval 组合） |
| `agents` / `agentLoop` / `agentPresets` | `dsh-agent` / `dsh-agent-loop` / `dsh-agent-preset-registry` | agent 生命周期、循环、preset 装配 |
| `sessions` / `compaction` / `spillStore` / `tokenMeter` / `toolResultPruner` | `dsh-session` / `dsh-compaction` / `dsh-spill` / `dsh-token-meter` / `dsh-compaction-tool-result-pruner` | 会话日志、压缩、溢出、token 计量、工具结果裁剪 |

其余：`llm`、`skills`、`subagents`、`commands`、`web`、`storage`、`workspaceRegistry`、`webServer`、`pluginManager`、`userQuestions`、`schedule`、`planMode`、`workflowEngine`、`otel` 等。

### 1.4 工具（Tool）如何注册与暴露给模型

1. 插件写工具：`ctx.tools.register(defineTool({...}))`
   —— 签名 `dsh-tools/lib/types/index.d.ts:50`；`defineTool` 定义在 `dsh-tools/lib/types/schema.d.ts`。
2. 注册进**作用域分层**：`ScopedLayers`（`dsh-scope/lib/types/store.d.ts:102-145`）
   - 全局层 + 每个 scope 一层；`merge()` 语义（`store.d.ts:127-133`）：
     "Materialize global named entries followed by scope-chain shadows, farthest ancestor first, so the nearest scope's entry wins a name."
   - `scopeChainOf(key)` 最近的 scope 在最后（`dsh-scope/lib/index.js:276-280`）。
3. schema 自动进入提示词组装：`ctx.systemPrompt.tools(...)`（`dsh-tools/lib/index.js:418-424`）。
4. 模型可见的 schema 由 executor 的 `describe()` 产出（`dsh-tools/lib/types/executor.d.ts`）。
   默认 executor 是 `dsh-tool-executor-native`（`dsh-tool-executor-native/lib/index.js:19-25`），
   另有 PTC 模式（`run_code` + 生成的 SDK），默认关闭（`dsh-base/cordis.patch.yml:501-506`）。

---

## 2. shell / bash 类工具：叫什么、参数 schema、有没有可插拔拦截点

### 2.1 工具名与参数

| 工具名 | 实现文件 | 平台 |
|---|---|---|
| `bash` | `dsh-tool-bash/lib/index.js:492-529` | 非 Windows（win32 下 disabled） |
| `pwsh` | `dsh-tool-pwsh/lib/index.js:465-500` | **Windows（本机当前生效）** |
| `bash`（persistent 变体） | `dsh-tool-bash-persistent/lib/index.js` | 备用实现 |
| `pwsh`（persistent 变体） | `dsh-tool-pwsh-persistent/lib/index.js` | 备用实现 |

`pwsh` 的参数 schema（`dsh-tool-pwsh/lib/index.js:468-501`）：

```yaml
description:   string, required   # UI 上显示的一句话说明
command:       string, required   # ← 要被改写的字段
timeoutMs:     number             # 覆盖默认超时
workdir:       string             # 工作目录（相对路径按 session cwd 解析）
run_in_background: boolean         # 后台任务
sandbox_permissions: string        # 枚举，沙箱升级用（仅沙箱 executor 存在时）
justification: string             # 配合 sandbox_permissions
```

`bash` 的 schema 同构（`dsh-tool-bash/lib/index.js:495-529`）。
注意：`cwd` **不是**参数名，是 `workdir`；`cwd` 由 `ctx.workingDirectory.ensure()` 解析（`dsh-tool-bash/lib/index.js:663-664`）。

### 2.2 执行路径（command 从模型到进程的完整链路）

```
模型 tool-call.arguments.command
  → pipeline 解析 + 参数深冻结        dsh-tools/lib/pipeline.js:327-331
  → tool.execute(args, exec)           dsh-tools/lib/pipeline.js:471
  → dsh-tool-pwsh: request.command = args.command
                                     dsh-tool-bash/lib/index.js:666-672
  → ctx.shell.resolve(request) → spec   dsh-tool-bash/lib/index.js:700-703
  → ctx.shell.execute(spec) → spawn     dsh-tool-bash/lib/index.js:704
```

### 2.3 关键结论：**没有「工具事件级」的入参改写点**

| 候选拦截点 | 能拿到 command？ | 能改写吗？ | 证据 |
|---|---|---|---|
| `tools/pre-execute` | ✅ `exec.arguments.command` | ❌ | `dsh-tools/lib/types/contracts.d.ts:318`："Input rewriting is excluded because arguments are already logged and presented."；`dsh-tools/README.md:197`："**`tools/pre-execute` deliberately cannot rewrite `exec.arguments`** — logged and rendered args would desync from what ran; the rewrite design is a proposed Agent Note (`.agents/notes/proposed/feature/2026-06-30-pre-tool-input-rewrite.md`)." |
| `tools/execute` | ✅ | ❌（只能换 signal） | `dsh-tools/lib/types/events.d.ts:28`："wrappers may change only `exec.signal`, while call identity remains immutable"；`dsh-tools/README.md:112`："Only the `tools/execute` view may replace the required signal" |
| `tools/post-execute` | ✅ | ⚠️ 只能改**结果** | `dsh-tools/lib/types/contracts.d.ts:341`（`PostToolDecision` 的 `accept{content\|value}` / `block{feedback}`） |
| `ctx.tools.guard()` | ✅ | ❌ 只能 deny | `dsh-tools/lib/types/contracts.d.ts:374`（`ToolGuard` 只返回拒绝原因） |
| 参数对象本身 | ✅ | ❌ 被冻结 | `dsh-tools/lib/pipeline.js:327-331`：`snapshotJsonValue` + `deepFreeze` |
| **`ctx.shell` 执行器** | ✅ | ✅ **可以** | `dsh-pwsh-local/lib/index.js:173-179`：`resolve(request){ … command: request.command … }`；`dsh-bash-local/lib/index.js:88-105` 同构 |

### 2.4 可行路径 A：替换 / 继承 shell 执行器（**推荐的 rtk 落点**）

`ctx.shell` 是所有 shell 类工具的唯一出口，且**官方明确支持子类化**：

- `dsh-bash-local/lib/index.js:5-12`：
  "Local Service Provider for the bash capability seam over the subprocess capability seam. … **subclasses may reuse the same mechanics with an explicit argv.** … **Execution policy belongs in `tools/pre-execute` or a sandboxing executor.**"
- `dsh-shell/lib/index.js:83-85`：`ShellExecutor extends Service`，`super(ctx, "shell")`；一个 context 只有一个实现。
- 沙箱变体本身就是子类：`dsh-pwsh-sandbox/lib/index.js:118`
  `class SandboxPwshExecutor extends PwshLocalExecutor`，并覆写 `resolve()`（`:145-147`，内部 `...super.resolve(request)`）。

于是「rtk 执行器」的最小实现约 20 行：

```js
// @local/dsh-shell-rtk/index.js
import { SandboxPwshExecutor } from '@deepseek-ai/dsh-pwsh-sandbox'

export class RtkPwshExecutor extends SandboxPwshExecutor {
  resolve(request) {
    const spec = super.resolve(request)          // 保留 workdir/timeout/stdin/env/dshEnv/sandboxPolicy
    return { ...spec, command: toRtk(spec.command) }   // ← 在 spawn 之前改写命令字符串
  }
}
```

装配方式：在 profile 的 `cordis.patch.yml` 里把 `pwsh-sandbox` 那一行的 `name` 换成 `@local/dsh-shell-rtk`。
（当前 profile：`C:\Users\maxu\.qianxun\dsh-home\profiles\web\cordis.patch.yml`；
base composition 里的 `pwsh-sandbox` 行见 `dsh-base/cordis.patch.yml:255-257`，win32 生效。）

**优点**：一次拦截覆盖 `pwsh` + `bash` + persistent 变体（都走 `ctx.shell`）；改写发生在 spawn 之前，语义上就是「命令重写代理」。

**注意**：DSH 内置的 `dsh-tool-pwsh` / `dsh-tool-pwsh-persistent` 自身是独立插件，
如果某个工具绕过 `ctx.shell`（例如 `dsh-terminal-bash` 直接用 `ctx.subprocess`），则覆盖不到——需单独核对。

### 2.5 可行路径 B：Agent scope 内同名工具「shadow」

机制上支持（`dsh-tools/lib/types/index.d.ts:44` "Scoped tools shadow globals"；
`dsh-tools/README.md:165` "Agent-scoped restrictions, shadows, and extension registrations change that agent's end-tool set."），
**但在本 profile 下不成立**：

- `pwsh` 工具本身是在 **agent preset** 里以 agent scope 装配的
  （`dsh-web-app/presets/standard.patch.yml` 的 `tool-pwsh` 行）。
- 同一个 scope 层内重名注册会直接抛错：
  `dsh-tools/lib/index.js:385` → `"tool \"pwsh\" is already registered in this scope"`
  （`NamedEntries.insert` 遇重名即抛，`dsh-scope/lib/index.js:27-30`）。
- preset 的 `plugins` 数组里再加一行同名工具 → 同层冲突。

**可行的变体**：不是「再加一个工具」，而是**把 preset 里 `tool-pwsh` 这一行的 `name` 换成你自己的插件**
（你的插件仍然 `defineTool({ name: "pwsh" })`）。这等于「自研一份 dsh-tool-pwsh 并改写 command」，
比路径 A（继承执行器）重，但能同时自定义工具描述 / 参数 / 呈现。

### 2.6 不可行的做法（明确排除）

- 在 `tools/pre-execute` 里 `exec.arguments.command = 'rtk ' + ...` → 参数已被 `deepFreeze`，赋值抛错或无效。
- 让 `tools/pre-execute` 返回 `{kind:'allow', arguments: ...}` → 类型里根本没有这个字段。
- 期望有类似 Claude Code `hookSpecificOutput.updatedInput` 的事件 → **不存在**（见 §3）。

---

## 3. 工具调用事件

### 3.1 DSH 的四个工具事件（全部在 `dsh-tools/lib/types/events.d.ts`）

| 事件 | 行号 | 模式 | 语义 |
|---|---|---|---|
| `tools/pre-execute` | `:17` | waterfall | 派发前闸门，返回 `PreToolDecision` = `allow` / `deny` / `cancel` / `ask` |
| `tools/execute` | `:28` | waterfall | 环绕派发（超时、重试、指标）；**只允许替换 `exec.signal`** |
| `tools/post-execute` | `:40` | waterfall | 接受 / 替换 / 阻断**结果**，可追加上下文 |
| `tools/result` | `:48` | emit | 只读观察冻结的最终结果 |
| `tools/change` | `:55` | emit | 工具注册变化 |

`PreToolDecision` 定义：`dsh-tools/lib/types/contracts.d.ts:321-340`。
`PostToolDecision` 定义：`dsh-tools/lib/types/contracts.d.ts:341-360`。

### 3.2 「异步返回修改后的参数」——不支持

- 没有 `beforeToolCall` / `tool_call` / `PreToolUse` 这类可携带「修改后入参」的事件。
- `dsh-session` 的会话事件表里确实有 `hook/invoked` / `hook/result`
  （`dsh-session/lib/types/known-event-types.js:41-42`），但它们属于
  `KNOWN_SESSION_EVENT_TYPES`——**「ignorable external session events」兼容机制**
  （`dsh-session/lib/index.js:99-100` 及其上方注释：为了兼容外部生产者的会话事件而保留的事件名注册）。
  DSH 0.2.2-alpha.3 **没有**任何 producer，也没有 `hookSpecificOutput` / `updatedInput` 结构。
- 全仓 330 个包 grep `PreToolUse` / `hookSpecificOutput` / `updatedInput` → **0 命中**。
- `dsh-shell/lib/types/types.d.ts:66,84` 提到 "the hooks bridges, which write a hook command's JSON payload to its stdin"、
  "the hooks bridges set `CLAUDE_PROJECT_DIR`, `CLAUDE_PLUGIN_ROOT`"——
  说明**上游 DSH 有过（或计划有）Claude Code 风格的 hooks bridge**，但**本 runtime 的 330 个包里没有对应包**，无法直接使用。

### 3.3 事件可达性

`tools/*` 事件都支持**作用域过滤**（`dsh-scope`）：
`scopeTarget(base, key)`（`dsh-scope/lib/index.js:327-338`）允许「祖先 scope 的监听器收到后代 scope 的事件」，
但**事件只往上流，不往下流**。所以 Host 级插件能观察到所有 agent 的调用，agent 级插件只能看到自己子树。

---

## 4. 插件 API

### 4.1 插件能注册什么

| 类别 | API | 例子 |
|---|---|---|
| Service | `class X extends Service` + `super(ctx, "name")` | `dsh-shell/lib/index.js:83` |
| Tool | `ctx.tools.register(defineTool({...}))` | `dsh-tools/README.md:45` |
| 工具作用域限制 | `ctx.tools.restrict({allow, deny})` | `dsh-tools/lib/types/index.d.ts:58` |
| 单调拒绝守卫 | `ctx.tools.guard(fn)` | `dsh-tools/lib/types/index.d.ts:136` |
| Executor | `ctx.tools.setExecutor(executor)` | `dsh-tools/lib/types/index.d.ts:117` |
| 提示词分段 | `ctx.systemPrompt.section({name, order, text})` | `dsh-tool-bash/lib/index.js:376-380` |
| 事件监听 | `ctx.on("tools/pre-execute", async (exec, next) => {...})` | `dsh-experimental-auto-review/lib/index.js:565` |
| 命令（斜杠命令） | `dsh-commands` 的 `ctx.commands` | — |
| UI slot（Client 侧） | `ctx.slots.provideRoot({...})` | `dsh-client-resources/lib/client.js:176` |
| Agent preset 行 | `dsh-agent-preset` 的一行声明 | `dsh-agent-preset/lib/index.js:8-30` |

### 4.2 注册自定义 Tool 的最小真实范例

摘自 `dsh-tools/README.md:38-61`（官方文档原样）：

```ts
import { readFile } from 'node:fs/promises'
import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'

declare const ctx: Context

ctx.tools.register(defineTool({
  name: 'read_file',
  description: 'Read a file from disk.',
  parameters: {
    path: { type: 'string', required: true, description: 'Absolute file path' },
    offset: { type: 'number' },
    limit: { type: 'number' },
  },
  output: {
    schema: { type: 'string' },
    render: (_args, value) => [{ type: 'text', text: value }],
  },
  async execute(args, exec) {
    // args is typed: { path: string; offset?: number; limit?: number }
    return readFile(args.path, { encoding: 'utf8', signal: exec.signal })
  },
}))
```

Host 插件的外壳（`dsh-agent-preset/skills/cordis-plugin-development/references/host-plugin.md:57-64`）：

```js
// index.js —— Host 插件导出二选一，不要混用
export function apply(ctx, config) {
  ctx.effect(() => ctx.tools.register(defineTool({ /* ... */ })))
}
export const inject = ['tools']        // 可选：依赖的服务
export const Config = z.object({ /* ... */ })   // 可选：配置 schema
```

bundle 声明（`host-plugin.md:9-27`）：

```json
{ "name": "@local/my-plugin", "version": "1.0.0", "private": true, "type": "module",
  "exports": { ".": "./index.js" },
  "dsh": { "bundle": { "patch": "./cordis.patch.yml" } } }
```
```yaml
# cordis.patch.yml
- insert:
    - id: my-plugin
      name: '@local/my-plugin'
      config: {}
```

### 4.3 「在执行前改写 command」的最小真实范例（本次新增推导）

```js
// @local/dsh-shell-rtk/index.js
import { SandboxPwshExecutor } from '@deepseek-ai/dsh-pwsh-sandbox'

const PREFIX = 'rtk '

export class RtkPwshExecutor extends SandboxPwshExecutor {
  /** 在 spawn 之前改写命令字符串；其余 spec 字段（workdir/timeout/stdin/env/sandboxPolicy）保持不变。 */
  resolve(request) {
    const spec = super.resolve(request)
    if (typeof spec.command !== 'string' || spec.command.trim() === '') return spec
    return { ...spec, command: PREFIX + spec.command }
  }
}
export default RtkPwshExecutor
```
装配：在 `cordis.patch.yml` 里把 `pwsh-sandbox` 行的 `name` 换成 `@local/dsh-shell-rtk`。

---

## 5. 权限 / 审批

### 5.1 默认没有「每条命令弹窗」

DSH 的权限模型是**沙箱模式**（`dsh-permission-presets/lib/types/index.d.ts:33-40`）：
每个预设是一个 `{ sandbox, approval }` 组合，sandbox 取值
`read-only` / `workspace-write` / `danger-full-access`，approval 取值 `ask` / `never`。

当前 profile 的预设（`C:\Users\maxu\.qianxun\dsh-home\profiles\web\cordis.patch.yml:117-130`）：

```yaml
- id: permission
  name: '@deepseek-ai/dsh-permission-presets'
  config:
    presets:
      read-only:           { sandbox: read-only,           approval: ask }
      workspace-write:     { sandbox: workspace-write,     approval: ask }
      danger-full-access:  { sandbox: danger-full-access,  approval: never }
    defaultPreset: danger-full-access
```

- 默认沙箱模式来自 `dsh-base/cordis.patch.yml:243-248`：`DSH_PERMISSION_MODE ?? 'workspace-write'`。
- `dsh-permission-presets/lib/index.js:180`：预设要求 `ctx.shell` 是**受限**执行器
  （`ctx.shell.sandboxMode !== undefined`），否则直接报配置错误。

### 5.2 审批服务

`ApprovalService`（`dsh-user-approval/lib/types/index.d.ts`）：
`request(req)` → `allowed-once` / `rejected` / `cancelled` / `unavailable`（**fail-closed**）；
会话策略 `approval/policy` ∈ {`ask`, `never`}。
只有 bash/pwsh 工具**申请沙箱升级**时才会走审批
（`dsh-tool-bash/lib/index.js:360-375` → `ctx.get("approval")`）。

插件可以自己在 `tools/pre-execute` 返回 `{kind:'ask'}` 触发审批，真实范例：
`dsh-experimental-auto-review/lib/index.js:565-598`（风险分类 → ask / deny / cancel）。

### 5.3 改写后的命令会不会被二次校验？——**不会被同等校验**

这是 rtk 集成必须注意的安全点：

1. `tools/pre-execute` / `tools/execute` 闸门发生在 **改写之前**（它们看到的是模型的原始 `arguments`）。
   如果改写发生在执行器层，**改写后的命令字符串从未经过任何内容级策略**。
2. shell 工具自带的校验只查「非空」与 `timeoutMs` 合法性：
   `dsh-tool-bash/lib/index.js:235-242`（`validateBashArgs`）/ `dsh-tool-pwsh/lib/index.js:249-250`。
   **没有命令内容白名单 / 黑名单**。
3. 沙箱策略按 agent/session 解析，**不看命令内容**
   （`dsh-tool-bash/lib/index.js:348` `resolveSandboxPolicy`），
   所以改写成 `rtk rm -rf …` 后，文件访问限制依然作用在 rtk 进程上（沙箱仍在）；
   但**命令语义层面的策略（若插件自己实现）需要自行在改写后重跑一遍**。

> 结论：DSH 没有「命令内容策略」这一层现成设施。若 rtk 插件要保留/新增这类策略，
> 必须在改写之后自行校验（否则等于绕过）。

---

## 6. 已有的「改写 / 压缩 / 统计 token」类插件

全仓 grep `rewrite` / `compress` / `truncate` / `token` / `filter` / `hook` 的结果（**没有任何改写命令的插件**）：

| 插件 | 机制 | 作用 | 位置 |
|---|---|---|---|
| `dsh-tool-call-timeout-policy` | `ctx.on("tools/execute")` | 替换 `exec.signal` 实现工具超时（**最贴近 rtk 插件形态的范例**） | `lib/index.js:116-141` |
| `dsh-session-checkpoint-policy` | `ctx.on("tools/execute")` | 派发前后 checkpoint | `lib/index.js:66` |
| `dsh-tool-fs-search` | `ctx.on("tools/post-execute")` | **替换结果内容**：超量匹配截断 + spill 到文件 | `lib/index.js:918-933` |
| `dsh-spill-policy` | `ctx.on("tools/post-execute")` | 大输出落盘 spill | `lib/index.js:237` |
| `dsh-repeat-tool-reminder` | `ctx.on("tools/post-execute")` | 重复工具调用提醒 | `lib/index.js:2252` |
| `dsh-experimental-auto-review` | `ctx.on("tools/pre-execute")` | 风险分类 → ask / deny / cancel | `lib/index.js:565-598` |
| `dsh-agent-instructions` | `ctx.on("tools/result")` | 观察工具结果 | `lib/index.js:1499` |
| `dsh-output-retention` | 纯库（无 Service / 无事件） | 文本 / 条目截断 + 溢出提示 | `lib/index.js`（`TextRetainer` / `ItemRetainer`） |
| `dsh-compaction-tool-result-pruner` | Service `toolResultPruner` | 工具结果**中段裁剪**，标记 `[... tool result middle pruned ...]` | `lib/index.js:8,91` |
| `dsh-token-meter` | Service `tokenMeter` | token 计量 / 上下文占用 | `lib/index.js` |

**结论**：
- 「**输出压缩**」有一等公民扩展点（`tools/post-execute` + `dsh-output-retention` + `dsh-compaction-tool-result-pruner`）——rtk 的压缩部分**不需要改写命令**就能做。
- 「**token 统计**」已有现成服务（`ctx.tokenMeter`）。
- 「**改写命令**」**没有任何现成实现**，必须自己写（路径 A 或 C）。

---

## 7. 结论：rtk 应该怎么接

### 路线 A（推荐）：继承 shell 执行器，在 `resolve()` 里改写 `command`

- 落点：`ctx.shell` 的子类，覆盖 `resolve(request)` 返回 `{...spec, command: 'rtk ' + spec.command}`。
- 覆盖范围：所有走 `ctx.shell` 的 shell 工具（`pwsh` / `bash` / persistent 变体）。
- 装配：profile `cordis.patch.yml` 里替换 `pwsh-sandbox`（win32）/`bash-sandbox`（其它平台）那一行的 `name`。
- 配套：`ctx.on("tools/post-execute")` 做输出压缩；`ctx.on("tools/result")` 做 token 统计；
  `ctx.systemPrompt.section({name:'tool:rtk', ...})` 告诉模型命令会被重写。
- 风险：改写后的命令**不再经过 pre-execute 的内容级策略**（DSH 本来也没有内容级策略，见 §5.3）；
  且 `dsh-tool-pwsh` 的 `description`（模型看到的工具说明）不会自动提到 rtk，需要额外用 system prompt section 补。

### 路线 B：自研一份 `dsh-tool-pwsh` 副本，注册同名工具 `pwsh`

- 落点：`defineTool({ name: 'pwsh', ... })`，在 `execute` 里改 `args.command`。
- 装配：把 agent preset（`dsh-web-app/presets/standard.patch.yml`）里的 `tool-pwsh` 行换成你的插件。
- 优点：能改工具描述、参数 schema、UI 呈现（`presentCall` / `presentResult`）。
- 缺点：代码量明显更大（要复刻 background job / promote / 沙箱升级 / 渲染逻辑，见 `dsh-tool-bash/lib/index.js:386-713`）。

### 路线 C：经典 shell 包装器（PATH shim）

- 在 PATH 前置放 `rtk.exe`（以及 `ls/cat/git` 等 shim），模型写 `ls`，shim 转 `rtk ls`。
- DSH 侧零代码；可用 `dsh-shell-env` / `ShellExecRequest.env`（`dsh-shell/lib/types/types.d.ts:85-95`）注入 PATH。
- 优点：完全不依赖 DSH 内部结构，跨 harness 通用；也能覆盖 `dsh-terminal-bash` 这类绕过 `ctx.shell` 的路径。
- 缺点：Windows 下 shim 需要 `.exe`/`.cmd` 且要抢在系统命令前面；不能改写复合命令（`a && b`）。

### 路线 D：提示词层引导（AGENTS.md / system prompt section）

- 让模型自己写 `rtk ls`。零代码、零风险，但**不强制**。
- 可与 A/B/C 叠加作为兜底。

### 最终判定（对照用户给的四个选项）

| 选项 | 判定 |
|---|---|
| (a) 插件在工具执行前改写命令字符串 | ✅ **成立**，但**不是**通过 `tools/pre-execute` / `tools/execute` 事件（官方明确禁止，`dsh-tools/README.md:197`），而是通过**继承 `ctx.shell` 执行器**（`dsh-pwsh-local/lib/index.js:173-179`、`dsh-bash-local/lib/index.js:88-105`）或**自研同名工具**（`dsh-tool-pwsh/lib/index.js:465`）。 |
| (b) 只能在提示词/AGENTS.md 层引导 | ✅ 成立，但最弱，只是兜底。 |
| (c) 只能包一层 shell 包装器 | ✅ 成立，是路线 C（PATH shim），与 (a) 并不互斥。 |
| (d) 完全不支持 | ❌ 不成立。 |

**推荐组合：A（执行器改写）+ `tools/post-execute`（输出压缩）+ C（PATH shim 兜底）+ D（提示词告知）**。

---

## 8. 未闭环项 / 需要在实现前确认

1. **`ctx.shell` 覆盖率**：本报告确认 `dsh-tool-bash` / `dsh-tool-pwsh` / `dsh-tool-*-persistent` 都走 `ctx.shell`；
   但 `dsh-terminal-bash`、`dsh-experimental-worktree`、`dsh-experimental-forge-session` 等直接用 `ctx.subprocess`，
   路线 A 覆盖不到它们，需要逐个核对是否值得覆盖。
2. **preset 与 base 双份 shell 工具行**：base composition（`dsh-base/cordis.patch.yml:281-287`）和 agent preset
   （`dsh-web-app/presets/standard.patch.yml`）都声明了 `tool-bash` / `tool-pwsh`；agent scope 的那份胜出
   （`dsh-scope/lib/types/store.d.ts:127-133`）。路线 A 改的是**执行器**（base 层），路线 B 改的是**工具**（preset 层）——
   两者作用点不同，实施时不要混淆。
3. **profile 补丁的覆盖语义**：patch 会**整体替换**目标行的 `config`
   （`dsh-agent-preset/skills/cordis-plugin-development/references/host-plugin.md:3`），
   所以覆盖 `pwsh-sandbox` 行时必须把 `config` 一起写全，否则会丢掉 base 里原有的 `timeoutMs` 等设置。
4. **本报告未做运行时验证**：全部结论来自源码静态阅读 + 包内容枚举，未在 `http://127.0.0.1:57230` 上跑通一次真实改写。
   下一步建议：做一个最小 `@local/dsh-shell-rtk` bundle，用 `pwsh` 工具跑一条 `Write-Output hi`，
   观察最终命令是否变成 `rtk Write-Output hi`。
5. **上游 hooks bridge 未落地**：`dsh-shell/lib/types/types.d.ts:66,84` 提到的 "hooks bridges" 在本 runtime 的 330 个包里不存在，
   若未来 DSH 补上 Claude Code 风格 hooks（含 `updatedInput`），路线 A 可平滑迁移到事件式改写。

---

## 附录：本次探查用到的可复现命令

```powershell
# 枚举所有 DSH 包及其落盘路径
python -X utf8 - <<'PY'
import os, json
base = r"C:\Users\maxu\.qianxun\dsh-runtime\node_modules\.pnpm"
for d in os.listdir(base):
    nm = os.path.join(base, d, "node_modules", "@deepseek-ai")
    if os.path.isdir(nm):
        for pkg in os.listdir(nm):
            print(pkg, os.path.join(nm, pkg))
PY

# 找工具事件监听者
python -X utf8 - "<repo>/.tmp/dsh-inspect/dshgrep.py" "tools/(pre-execute|execute|post-execute|result)"
```
