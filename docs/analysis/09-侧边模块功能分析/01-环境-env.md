# 侧边模块分析报告 · 环境（env）

> 分析日期：2026-10-05 ｜ 范围：前端 `src/features/env/EnvPage.svelte`、`src/stores/harness.svelte.ts`、`src/lib/ipc/contract.ts`（harness/tool 段）；后端 `src-tauri/src/harness/**`（8 文件）、`src-tauri/src/child_output.rs`、`src-tauri/src/single_instance.rs`
> 负责：worker / env-web-dsh-module

## 0. 速览

| 项         | 值                                                                                                                                                      |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 一句话定位 | 本机 AI 运行时控制台：四张卡（DSH 托管进程 / Laya 决策引擎 / Qwen3.8-Flash-Next 本地模型 / Moli 无头浏览器）各自的启停与配置，加一块合并日志面板        |
| 完成度     | **84/100（良好，主链路闭环）**                                                                                                                          |
| 代码规模   | 前端 927 行（`EnvPage.svelte` 713 + `harness.svelte.ts` 214） / 后端 ~3448 行（harness 8 文件 3173 + `child_output.rs` 135 + `single_instance.rs` 138） |
| 最严重问题 | 环境页把 DSH 显示为绿色「运行中」，但 DSH 页能否用取决于**远程域的回环网关**——网关挂了环境页毫无察觉（见 §4-E1）                                        |

### ⚠️ 首要结论：任务简报中「多标签终端」的定位已过期

任务简报把 `env` 描述为「多标签终端（conpty / xterm / WebGL 渲染）」。**代码中不存在任何终端**：

- `src/` 与 `src-tauri/src/` 下 `terminal` 字样命中数均为 **0**；
- `package.json` 无 `@xterm/*` 依赖，无 `portable-pty`；
- `docs/02-技术架构.md:154-157` 明确记载：「终端域已在 v0.5 **整体移除**（Rust `terminal/`、前端 `features/terminal/`、portable-pty 与 xterm 三件套依赖、settings.terminal 偏好、PIN 记录 IPC 全删）」；
- `docs/05-功能深化设计-v0.2.md:12` 记载了移除原因：Rust 会话 id 从 0 起始而前端渲染门 `{#if tab.id > 0}` 把 0 号会话吞掉，导致「有标签条、无终端、无报错」，随后该页整体退役；
- 仍写着「终端」的 `docs/01-产品设计.md:40`、`docs/04-里程碑与TODO.md:97-105`、`README.md:22` 是**未同步的历史文档**（见 §4-E9）。

`window.rs:2` 的模块注释「独立窗口（终端 / DSH 页分离到独立窗口）」与 `capabilities/standalone-windows.json:4` 的描述「独立窗口（终端/DSH 分离窗）」同样是移除前遗留。**因此本报告不含任何终端/conpty/xterm/句柄泄漏分析**——那套代码已不存在，无从分析。若产品意图是恢复终端，那是新功能而非缺陷修复。

`env` 的真实定位是**侧边第 1 项，也是整个 DSH 生态的启动入口**（`nav.page` 初始值即 `'env'`，`src/stores/nav.svelte.ts`），因为 DSH 页、联网页、笔记桥、插件栈全部依赖它拉起的 DSH 进程。

---

## 1. 模块定位与设计意图

### 1.1 四张卡的分层

`EnvPage.svelte` 用一个 `flex` 布局承载四张语义不同的卡（`:301` 左侧 24rem 滚动栏 + `:719` 右侧日志铺满）：

| 卡                 | 位置       | 职责                                                   | 后端            |
| ------------------ | ---------- | ------------------------------------------------------ | --------------- |
| DSH                | `:303-444` | 托管 `dsh web` 进程：环境探测 → 安装 → 启停 → 崩溃自愈 | `harness/**`    |
| Laya               | `:447-551` | 决策引擎 sidecar（onnx 推理，端口 10230）              | `tools/laya.rs` |
| Qwen3.8-Flash-Next | `:554-639` | 本地模型服务（Strata 引擎，载入 1-3 分钟）             | `tools/qwen.rs` |
| Moli               | `:642-715` | 无头浏览器二进制探测/安装（`web_fetch` 的执行体）      | `tools/moli.rs` |

三张工具卡的共同模式值得称道：**状态与配置分离，配置存 settings，状态走 IPC**，且三者都区分「千寻自有子进程」与「用户自己起的外部实例」——`layaStatus.pid` 为空时前端显示「外部实例」（`:271`），`qwenStatus.ready` 与 `running` 分开（`contract.ts:242-252`），`moliStatus.source` 有四态枚举（`contract.ts:619`）。这是真实使用过才会写出的代码。

### 1.2 DSH 卡的「启动计划」设计

`harness/mod.rs` 把「这台机器能跑什么」与「怎么跑起来」拆成两个纯函数，这是本模块最清晰的设计决策：

- `environment()`（`mod.rs:157-197`）——纯探测：找出所有 Node 运行时（千寻自管的与用户装的走**同一条排序规则**）、判定 DSH 是否安装、版本是否等于 `PINNED_VERSION`；
- `launch_plan_for()`（`mod.rs:219-279`）——把探测结果变成 `LaunchPlan`，或返回**可诊断的错误**（`Error::NoNodeRuntime` / `DshNotInstalled` / `DshVersionMismatch`）；
- `install_plan()`（`mod.rs:282-319`）——安装计划，额外要求那个 Node 配套的 `npm`（`:296-309`：「一个只有 Node 的更新安装不能把一个带 npm 的完整旧安装挤掉」）。

三处诊断日志（`mod.rs:232-242`、`249-257`）把判定依据原样落盘，环境问题一眼可辨——这是本仓库一贯的「错误信息即文档」风格。

### 1.3 ADR-015：版本闸门

`dsh_version_matches` 为 false 时**拒绝启动**（`mod.rs:247-262`），UI 显示「版本不匹配：要求 X，当前 Y」并要求用户主动点重装（`EnvPage.svelte:423-439`）。这是有意的硬闸门：不让未经适配验证的 DSH 版本继续跑。

---

## 2. 功能清单与完成度

### 2.1 DSH 卡

| #   | 功能                         | 状态         | 证据(file:line)                                                | 备注                                          |
| --- | ---------------------------- | ------------ | -------------------------------------------------------------- | --------------------------------------------- |
| 1   | 状态点着色（5 相位）         | 已完成       | `EnvPage.svelte:83-89, 307-310`                                | 5 相位全覆盖                                  |
| 2   | 启动 / 停止 / 重启           | 已完成       | `:314-342`；`harness.svelte.ts:135-160`                        | 重启走 `harness_restart`（停→等→启）          |
| 3   | 启动按钮三重前置校验         | 已完成       | `:331-336`                                                     | 缺 Node / 未装 DSH / 版本不匹配 均禁用        |
| 4   | 一键装 Node                  | 已完成       | `:394-408`；`harness_install_node` `commands.rs:151-184`       | 按钮文案读 `bundledNodeVersion`，前端不硬编码 |
| 5   | 一键装 / 升 DSH              | 已完成       | `:409-440`；`perform_install` `commands.rs:186-222`            | 事务安装（备份→直装→校验→清理）               |
| 6   | 失败态「恢复到上次可用配置」 | 已完成       | `:354-369`；`harness_recover_known_good` `commands.rs:380-401` | 仅 `snapshotAvailable` 时显示                 |
| 7   | 失败态「安全模式」           | 已完成       | `:370-384`；`harness_safe_mode_start` `commands.rs:406-423`    | 逃生舱，默认 profile 一字节不动               |
| 8   | 安全模式徽标                 | 已完成       | `:386-393`                                                     | `data-testid="safe-mode-badge"`               |
| 9   | 安装进度可视化               | **占位骨架** | 数据齐备但 UI 不渲染                                           | 见 §4-E4                                      |
| 10  | 显示 DSH 回环代理地址        | **未实现**   | 环境页无任何 proxy 字段                                        | 见 §4-E1                                      |
| 11  | 显示带 token 的 DSH URL      | 未实现       | `harness.status.url` 从未进 UI                                 | 仅 DSH 页内部使用                             |

### 2.2 Laya / Qwen / Moli 三卡

| #   | 功能                 | 状态     | 证据(file:line)                                   | 备注                                          |
| --- | -------------------- | -------- | ------------------------------------------------- | --------------------------------------------- |
| 12  | Laya 启停 + 四项配置 | 已完成   | `:447-551`；`laya_start/stop` `contract.ts:87-88` | 端口/线程有 `min`/`max` 约束                  |
| 13  | Laya 未配置原因区分  | 已完成   | `:482-490`                                        | 分「可执行文件未找到」/「模型目录未找到」     |
| 14  | Qwen 启停 + 三项配置 | 已完成   | `:554-639`                                        | 启动按钮要求 `strataHome` 非空（`:572`）      |
| 15  | Qwen 载入中状态文案  | 已完成   | `:283-291`                                        | 「内存载入 1-3 分钟，期间卡顿属正常」         |
| 16  | Moli 三源解析展示    | 已完成   | `:119-124, 678-692`                               | 自备/受管/PATH/未安装                         |
| 17  | Moli 重新检测        | 已完成   | `:656-668`                                        | 保存后自动复检（`:210-211`）                  |
| 18  | 三卡状态轮询         | 部分完成 | `:221-232`                                        | **只轮询 Laya+Qwen，Moli 不轮询**（见 §4-E3） |

### 2.3 日志面板

| #   | 功能                    | 状态       | 证据(file:line)                         | 备注                            |
| --- | ----------------------- | ---------- | --------------------------------------- | ------------------------------- |
| 19  | DSH 日志 + 侧车日志合并 | 已完成     | `:239` `[...harness.logs, ...procLogs]` | 侧车行自带 `[tool.stream]` 前缀 |
| 20  | 双端 2000 行环形上限    | 已完成     | `:236`；`harness.svelte.ts:17, 94`      | 两侧独立封顶                    |
| 21  | 只渲染尾部 150 行       | 已完成     | `:739` `slice(-150)`                    | 注释点明全量渲染是卡顿源        |
| 22  | 滚动到底部钉住          | 已完成     | `:241, 243-246, 729-732`                | 24px 阈值判定，用户上滑即解除   |
| 23  | 历史回填                | 已完成     | `harness.svelte.ts:221-229`             | 晚订阅不空白                    |
| 24  | 日志导出 / 复制 / 过滤  | **未实现** | 无                                      | 排障需手工截屏                  |

### 2.4 后端状态机

| #   | 功能                           | 状态               | 证据(file:line)                             | 备注                                       |
| --- | ------------------------------ | ------------------ | ------------------------------------------- | ------------------------------------------ |
| 25  | 5 相位状态机 + 事件广播        | 已完成             | `supervisor.rs:66-92, 501-504`              | `phase` + `kind` 双 tag，serde rename 齐全 |
| 26  | 内联首次启动（错误不藏进重试） | 已完成             | `supervisor.rs:194-235`                     | 设计意图写在 `:191-193`                    |
| 27  | 就绪行解析 + 回环强校验        | 已完成             | `readiness.rs:28-53`                        | 非回环/非 http/无端口一律 `Rejected`       |
| 28  | 120s 有界就绪等待              | 已完成             | `supervisor.rs:28, 305-308`                 | 注释：紧阈值惩罚慢磁盘而非真故障           |
| 29  | 启动失败附 stderr 尾部         | 已完成             | `supervisor.rs:35, 324-336, 536-541`        | 160 行 + 1s drain 超时                     |
| 30  | 崩溃退避重启（5 级）           | 已完成             | `supervisor.rs:31, 474-499`                 | 500/1s/2s/5s/10s，用完转 `Failed`          |
| 31  | HTTP 健康看门狗                | 已完成             | `health.rs:21-64`；`supervisor.rs:445-471`  | 真发 HEAD，任何状态码算健康                |
| 32  | 连续 3 次未命中才判死          | 已完成             | `supervisor.rs:46, 459-468`                 | 每次命中清零                               |
| 33  | 进程树归属（不碰用户进程）     | 已完成             | `supervisor.rs:16, 240-242`；`ProcessGuard` | ADR-011                                    |
| 34  | 生命周期闸门（串行化启停装）   | 已完成             | `commands.rs:23, 96, 110, 188, 240`         | `lifecycle: Mutex<()>`                     |
| 35  | 安装互斥（防双击双 npm）       | 已完成             | `commands.rs:21, 123-125, 153-155`          | `installing: AtomicBool`                   |
| 36  | 停机后等监督循环退出           | 已完成             | `supervisor.rs:248-256`；5s 上限            | 防 rename 撞活进程                         |
| 37  | known-good 快照（原子写）      | 已完成             | `recovery.rs:53-69`                         | 临时名 + rename；非法 JSON 拒绝快照        |
| 38  | 启动失败一级自动自救           | 已完成             | `commands.rs:264-318`                       | 快照相同则不重试（有诊断价值）             |
| 39  | 最小安全 profile               | 已完成             | `recovery.rs:18-31, 161-209`                | 所有权标记，**拒绝吞并用户同名目录**       |
| 40  | 中断安装自动恢复               | 已完成             | `mod.rs:158-165`                            | 无标记时零写入空操作                       |
| 41  | 超长子进程行截断               | 已完成             | `child_output.rs:9-10, 13-58`               | 32KB 上限，超长仍排水不阻塞退出            |
| 42  | 路径级单实例                   | 已完成             | `single_instance.rs:93-100`                 | exe 路径 + 标识符 FNV-1a，dev/安装版可并存 |
| 43  | `settings.dsh.port` 生效       | **未实现（有意）** | `mod.rs:270-275` 硬编码 `port: 0`           | 注释声明为历史字段；UI 未暴露              |

---

## 3. 实现分析

### 3.1 架构与数据流

```
EnvPage.svelte ──onMount──► harness.refreshEnvironment()   → harness_environment (spawn_blocking)
    │                       harness.backfillLogs()        → harness_log
    │                       harness.refreshRecovery()      → harness_recovery_status
    │                       listen('env://log')            → tools/mod.rs 侧车输出
    └─ $effect(10s) ──────► laya_status / qwen_status

harness.svelte.ts（单例 store，事件只建一次）
    listen('harness://event')     ──► ingest()   状态机镜像 + 日志环形缓冲
    listen('harness://install-progress') ──► ingestProgress()  ← UI 未消费
         ▲                                    ▲
         │                                    │
   Supervisor::publish / record          progress_sink (mod.rs:74-79)
         ▲
   start_locked → launch_plan_for → Supervisor::start → launch_once
                                                    → supervise → watch_health / revive
```

一个值得注意的耦合：`harness_environment` 被放进 `spawn_blocking`（`commands.rs:60`），因为探测会 spawn 多个 `node --version`。这是对的——同步 IO 不该占异步线程池。

### 3.2 前后端契约

`contract.ts:522-612` 定义了 harness 域全部类型，与 Rust 侧 `mod.rs:134-153`（`Environment`）、`supervisor.rs:66-84`（`Status`）、`mod.rs:49-71`（`InstallProgress`）字段级对齐。

两个易错点处理得很到位：

1. **`rename_all_fields = "camelCase"`**（`mod.rs:47`、`supervisor.rs:64`）——`InstallProgress::DshPackages` 的 `total_hint`/`downloaded_bytes` 等 snake_case 字段必须显式转换，否则前端拿不到驼峰键。`AGENTS.md` §6 与 `docs/07` 都点名过这个坑。
2. **可选进度维度**：`DshPackages` 的 `resolved`/`total_hint` 是 `Option<u64>`，pnpm 每行只带部分维度。前端 `ingestProgress`（`harness.svelte.ts:102-113`）在同 stage 时用 `??` 沿用上次的值，**展示层免判空**——这是个很小但很体贴的处理。

`formatHarnessStatus`（`contract.ts:574-587`）把 5 相位映射成中文，穷尽且无 `default` 分支（`switch` 覆盖联合类型全部成员，新增相位会编译报错）。

### 3.3 资源与生命周期

| 资源                 | 归属                      | 释放路径                                                                     | 评价                                   |
| -------------------- | ------------------------- | ---------------------------------------------------------------------------- | -------------------------------------- |
| DSH 子进程树         | `ProcessGuard`            | `guard.finish(pid)` 逐次 + `terminate_all()` + `Drop`                        | ✅ 孤儿进程有硬保证                    |
| stdout/stderr 泵任务 | `tokio::spawn`            | 管道 EOF 自然结束；stderr 超时则 `abort()`（`supervisor.rs:332`）            | ✅ 无 detach 泄漏                      |
| 安装期状态簿记       | `Mutex`                   | —                                                                            | ✅ 中毒恢复（`supervisor.rs:526-533`） |
| 前端日志缓冲         | 数组 `splice`             | 硬上限 2000×2                                                                | ✅ 有界                                |
| 事件监听             | `listen().then(unlisten)` | `onDestroy`（`EnvPage.svelte:36`）/ `dispose()`（`harness.svelte.ts:71-77`） | ⚠️ 见 §4-E2                            |
| 子进程单行           | `child_output::next_line` | 32KB 截断                                                                    | ✅ 有界                                |

`Supervisor::drop`（`supervisor.rs:559-565`）只置 `stopping`，不杀进程——注释说「进程树本身的回收是 guard 的职责」。这个职责划分是对的。

### 3.4 错误处理与恢复

**四级恢复链**，从自动到手动，层次清晰：

1. **进程级**：崩溃 → 退避重启 5 次（`supervisor.rs:474-499`）
2. **服务级**：哑掉（端口通但不回 HTTP）→ 健康看门狗终结进程 → 走同一条退避路径（`supervisor.rs:409-413`）
3. **配置级**：启动失败且快照与当前 manifest 不同 → 自动恢复重试一次（`commands.rs:264-318`）
4. **用户级**：手动「恢复上次可用配置」/「安全模式」（`commands.rs:380-423`）

第 2 级尤其值得称道：`watch_health` 命中后**不直接重启，而是先 kill 让恢复走唯一的退避路径**（`supervisor.rs:407-408` 注释）——避免出现两条并行的恢复逻辑。

`Error::Readiness` 会把 stderr 尾部拼进错误消息（`supervisor.rs:536-541`），这个错误最终显示在 DSH 页的失败卡片里（`DshPage.svelte:159-163`）——**环境页的失败原因与 DSH 页的失败原因共享同一个字符串**，排障链路是通的。

---

## 4. 缺陷清单

| 级别   | 问题                                                                                                                                                             | 位置                                                                                                   | 影响                                                                                                                                                                                                      | 证据                                                                                                       |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| **P1** | 环境页的 DSH 绿灯不代表 DSH 页可用。`harness_proxy_url` 实际读的是**远程域**的网关句柄（`state.remote.running`），与 supervisor 状态完全解耦                     | `harness/commands.rs:76-85`                                                                            | 网关启动失败时：环境页显示「运行中 · origin」绿色，DSH 页却停在「回环代理尚未监听」的死胡同。用户在环境页找不到任何线索，失败原因只在 DSH 页                                                              | `commands.rs:76-85` vs `EnvPage.svelte:345-350`（只渲染 `origin`）与 `DshPage.svelte:95-108`（代理缺失页） |
| **P1** | 健康看门狗会误杀长回合。`HEALTH_INTERVAL=10s` × `HEALTH_MISS_LIMIT=3`（单次超时 5s）→ DSH 约 30-45s 无 HTTP 应答即被 kill + 重启                                 | `supervisor.rs:39, 42, 46, 445-471`                                                                    | 若 DSH 的模型回合/工具调用同步阻塞事件循环超过 ~45s，会被判定为「哑掉」而重启，**正在进行的会话被硬打断**。注释（`supervisor.rs:44-45`）承认此风险但只按「忙是正常的」放宽到 3 次，未针对「长回合」做豁免 | `supervisor.rs:44-46, 409-413`                                                                             |
| **P2** | 安装进度数据齐备但 UI 不渲染。Rust 有 Node 下载的**字节级**进度，store 有完整的合并逻辑，环境页只展示文本日志                                                    | `harness/mod.rs:49-71`；`harness.svelte.ts:33, 97-115`；`EnvPage.svelte`（0 处引用 `installProgress`） | 30MB+ 的 Node 下载与数百包 pnpm 安装期间，用户只能在原始日志里翻 `Progress: resolved …`。已验证：`installProgress` 在所有 `.svelte` 中命中数为 **0**                                                      | 注释自认「前端 store 留存备用；环境页 UI 只展示日志」（`mod.rs:34`）                                       |
| **P2** | `env://log` 监听不随页面可见性开关，而同文件的轮询 `$effect` 却做了可见性门控——同一页面两种策略                                                                  | `EnvPage.svelte:26-32` vs `:221-232`                                                                   | keep-alive 下环境页一旦访问过就常驻，监听永不解除。Laya/Qwen 侧车此后每行输出都会 push 进 `procLogs` 并触发隐藏页的 `$derived` 数组重算（`:239` 每次复制 2000+ 元素）                                     | `App.svelte:89-92`（keep-alive）+ `EnvPage.svelte:26-32`                                                   |
| **P2** | Moli 状态不参与轮询，只能手动「重新检测」                                                                                                                        | `EnvPage.svelte:221-232`（只有 laya/qwen）、`:656-668`                                                 | 用户在千寻之外装好/删掉 Moli 后，环境页永远显示旧状态，直到手动点击或重启应用。与另两张卡的行为不一致                                                                                                     | `EnvPage.svelte:221-232`                                                                                   |
| **P2** | 日志合并用 `$derived([...a, ...b])`，每来一行都产生新数组身份                                                                                                    | `EnvPage.svelte:239`                                                                                   | 两个各 2000 行的数组在每条日志到达时整体复制。DSH 冷启动刷屏时是 O(n) 分配/行，叠加 §4-P2 的隐藏页常驻问题，浪费翻倍                                                                                      | `EnvPage.svelte:239, 236`                                                                                  |
| **P2** | 日志无过滤/无导出                                                                                                                                                | `EnvPage.svelte:718-744`                                                                               | 2000 行混排 DSH 与三张侧车的输出，无法按 `tool`/`stream` 过滤，也无法复制某段给排障。排障只能靠截图                                                                                                       | `EnvPage.svelte:718-744`                                                                                   |
| **P3** | `backfillLogs` 与实时事件无去重标记                                                                                                                              | `harness.svelte.ts:221-229`                                                                            | 冷启动 DSH 自动拉起时，历史回填与实时事件可能重叠，同一批行渲染两次（`logs.length > 0` 的守卫只挡住「已有日志就不回填」，挡不住「回填途中事件到达」）                                                     | `harness.svelte.ts:222-225`                                                                                |
| **P3** | `harness.starting` 只在 catch 或 ready/failed/stopped 事件里清零                                                                                                 | `harness.svelte.ts:135-143, 86-87`                                                                     | 若启动期间事件订阅被拆（`dispose()`），`starting` 永不复位 → 启动按钮永久禁用。当前只在应用拆卸时拆，风险低                                                                                               | `harness.svelte.ts:86-87, 135-143`                                                                         |
| **P3** | `settings.dsh.port` / `allowRandomFallback` 是死设置                                                                                                             | `mod.rs:270-275`；`contract.ts:162-168`                                                                | schema 里仍在，注释声明为历史字段。**已验证设置 UI 未暴露这两个字段**，所以用户改不到；但它们仍随备份导出/导入流动。清理成本低收益低                                                                      | `mod.rs:270-275`                                                                                           |
| **P3** | 二次启动只亮窗，不转发 argv                                                                                                                                      | `single_instance.rs:16, 113-127`                                                                       | 未来做 deep-link 唤起 DSH/打开某页时需要升级。注释已明确记录该限制                                                                                                                                        | `single_instance.rs:16`                                                                                    |
| **P3** | 文档漂移：`docs/01-产品设计.md:40`、`docs/04-里程碑与TODO.md:97-105`、`README.md:22`、`window.rs:2`、`capabilities/standalone-windows.json:4` 仍描述已删除的终端 | 见左                                                                                                   | 新读者（含本报告的委托方）会按「多标签终端」去找不存在的代码，浪费排查时间                                                                                                                                | 已验证 `src/`+`src-tauri/src/` 中 `terminal` 命中 0                                                        |

---

## 5. 改善建议

按 收益/成本 排序。

| 优先级 | 建议                                                                                                                                                          | 改动文件                                                                                                                                       | 收益                                                                                              | 成本                                                                                     |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| 1      | **DSH 卡补一行代理状态**：新增 `harness_proxy_url` 的轮询（并入现有 10s `$effect`），未就绪时在 DSH 卡显示「回环网关未监听，DSH 页暂不可用」+ 跳转 DSH 页按钮 | `src/features/env/EnvPage.svelte`（`:221-232` 扩为三方轮询、`:345-350` 加一行）                                                                | 高——直接消除 P1-E1 的「绿灯但不可用」类最难排查的故障；顺带覆盖 Moli 不轮询（P2）                 | 极低（~15 行，复用已有 store 字段）                                                      |
| 2      | **渲染安装进度条**：把 `harness.installProgress` 接进 DSH 卡（Node 下载用 `downloadedBytes/totalBytes` 百分比，DSH 用 `added/totalHint`）                     | `src/features/env/EnvPage.svelte`                                                                                                              | 高——数据链路已全部就绪（Rust 事件 → store），只差展示；长安装的等待体验从「翻日志」变为「有进度」 | 低（~30 行纯展示）                                                                       |
| 3      | **给健康看门狗加长回合豁免**：DSH 侧在跑模型回合/工具调用时打一个「忙」信号（或让 `/health` 在忙时立刻回 200），`watch_health` 见到忙信号就清零 misses        | `src-tauri/src/harness/health.rs`（探测路径）+ `supervisor.rs:445-471`                                                                         | 高——消除 P1-E2 的会话硬打断；当前阈值是拍的，没有实测数据支撑                                     | 中（需与 DSH 侧约定一个信号；无此信号时只能退化为「把阈值调到 5 分钟」这种更粗糙的方案） |
| 4      | `env://log` 监听与轮询统一策略：监听也加 `nav.page === 'env'` 门控，或反过来给轮询/监听都加一个统一的 `pageVisible` 派生量                                    | `src/features/env/EnvPage.svelte:26-32`                                                                                                        | 中——消除隐藏页常驻开销                                                                            | 低                                                                                       |
| 5      | 日志面板加 `stream`/`tool` 过滤 chip + 「复制全部」按钮                                                                                                       | `src/features/env/EnvPage.svelte:718-744`                                                                                                      | 中——排障效率                                                                                      | 低                                                                                       |
| 6      | 修文档漂移：删/改 5 处「终端」描述                                                                                                                            | `docs/01-产品设计.md`、`docs/04-里程碑与TODO.md`、`README.md`、`src-tauri/src/window.rs:2`、`src-tauri/capabilities/standalone-windows.json:4` | 中——防止后续 agent 重复本次的排查成本                                                             | 极低                                                                                     |
| 7      | 清理死设置 `dsh.port` / `allowRandomFallback`                                                                                                                 | `src-tauri/src/settings.rs`、`src/lib/ipc/contract.ts:162-168`                                                                                 | 低——减少误导                                                                                      | 低（注意有 settings schema 迁移面）                                                      |
| 8      | 合并日志改为「单一数组 + 来源标记」而非每行拼两个数组                                                                                                         | `src/features/env/EnvPage.svelte:239`                                                                                                          | 低——省掉每行 O(n) 复制                                                                            | 低                                                                                       |

---

## 6. 完成度评估

| 维度              | 得分   | 依据                                                                                                                                                                                                                                                                  |
| ----------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 功能完整性        | 82     | 四张卡的核心闭环（探测→安装→启停→配置→自愈）全部可用。缺：安装进度展示、代理状态可见性、日志过滤                                                                                                                                                                      |
| 健壮性 / 错误恢复 | 94     | **本仓库最强的一块**。四级恢复链、进程树归属、32KB 行截断、中毒恢复、事务安装、启动自愈、已知限制（conpty 式句柄问题在此不适用）均有单测覆盖                                                                                                                          |
| 性能与资源        | 84     | 进程/句柄/缓冲全部有界且有明确上限。扣分在隐藏页常驻监听 + 每行 O(n) 数组复制 + 轮询未覆盖 Moli                                                                                                                                                                       |
| 测试覆盖          | 80     | harness 40 个 Rust 单测全绿（readiness 解析、端口校验、参数次序、快照/安全 profile、幂等迁移…）。扣分：**Node 真实下载链路是 `#[ignore]`**（`harness::node_install::tests::真实网络下载与解压链路`，注释「真实网络下载，约 30MB」）；前端 store 与 EnvPage **零单测** |
| UX                | 80     | 状态点、失败原因、诊断日志、可诊断错误文案都很好。扣分在长安装无进度、网关故障无提示、日志不可用                                                                                                                                                                      |
| 可维护性          | 86     | 分层干净（探测/计划/执行/状态机/自愈五层分离），注释写「为什么」而非「是什么」，serde 契约锁死。扣分在三张工具卡逻辑堆在同一个 713 行组件里                                                                                                                           |
| **加权总分**      | **84** | 评级 **B+ / 良好**                                                                                                                                                                                                                                                    |

**为什么不是更高**：这是一个「后端优秀、前端壳子薄」的模块。Rust 侧的工程质量明显高于前端——前端有 3 个 P2 级的「数据已就绪但没接线」问题（安装进度、代理状态、Moli 轮询），本质上是同一类欠账。

**为什么不是更低**：P1 只有两条且都不阻断主链路（DSH 页仍可用 / 长时间无应答才触发），核心的「DSH 跑不起来」场景有四级恢复链兜底，并有 40 个通过的 Rust 单测。

---

## 7. 测试与验证

**实际执行**（工作目录 `E:\develop\dsh-workspace\qianxun`）：

| 命令                                                                                      | 结果                                                                                                                                                                                                                                                                                   |
| ----------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib harness`                           | **40 passed / 0 failed / 1 ignored**（2.01s）。唯一 ignored：`harness::node_install::tests::真实网络下载与解压链路`，测试名自带说明「真实网络下载，约 30MB」                                                                                                                           |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib`（全量）                           | **167 passed / 0 failed / 5 ignored**（2.05s）。harness 域内 ignored 仅上述 1 个；其余 4 个 ignored 中我只定位到 `search::commands::tests::真实盘符根可作搜索根`（属搜索域，与本模块无关），另 3 个的名字未被本次 tailed 输出捕获                                                      |
| `npx vitest run src/lib/external-links.test.ts src/stores/web.svelte.test.ts src/lib/ipc` | **3 files / 28 tests 全部 passed**（384ms）。本模块直接命中的只有 `contract.test.ts` 里的 IPC 命令名/事件通道一致性断言                                                                                                                                                                |
| `npx vitest run`（全量前端单测）                                                          | 未单独执行；上述 3 个文件已覆盖本模块全部前端测试面（本模块无专属 `.test.ts`）                                                                                                                                                                                                         |
| PowerShell 静态核查                                                                       | `Select-String 'terminal'` 在 `src/**` 与 `src-tauri/src/**/*.rs` 命中数均为 **0**；`installProgress` 在 `src/**/*.svelte` 命中数 **0**；`scheduleSearch` 在 `src/**` 命中数 1（仅定义处）；`dsh.port`/`allowRandomFallback` 在 `src/features/settings` 命中数 **0**（设置 UI 未暴露） |
| 行数统计                                                                                  | `Get-Content \| Measure-Object -Line`，用于 §0 代码规模                                                                                                                                                                                                                                |

**未执行**（如实标注）：

- `pnpm e2e` / `pnpm e2e:build` —— 需要先构建 debug 二进制并要求本机存在真实 DSH 安装。`e2e/probe-env-page.ts` 与 `e2e/probe-env-attach.ts` 是手写探针脚本，**不在 vitest/playwright 的自动化套件内**，本次未运行。因此环境页的真实渲染、DSH 真实启停、崩溃恢复、Node 真实下载**均未经真机验证**。
- `cargo clippy` / `cargo fmt --check` —— 未运行。
- `pnpm check`（prettier + eslint + svelte-check）—— 未运行全量；本报告写入后曾对本文件单独跑 prettier 校验（见 §7 补记）。
- **未逐行审读**：`src-tauri/src/harness/install.rs`（812 行）、`node_install.rs`（434 行）、`src-tauri/src/tools/{laya,qwen,moli}.rs`（共 36.8KB）。对它们的判断仅来自：调用点（`EnvPage.svelte` / `contract.ts`）、测试名与测试通过情况、以及模块级注释。§2.2 中三张工具卡标为「已完成」的结论**可信度低于** §2.4 的 supervisor 部分。

**补记（本报告自身格式校验）**：`npx prettier --check` 对本文件**初次未通过**（表格列宽对齐问题），已执行 `npx prettier --write` 仅格式化本文件后复检通过 → `All matched files use Prettier code style!`。写入范围仅限本文件，未触碰同目录其他 worker 的报告。
