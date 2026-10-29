# 侧边模块分析报告 · 插件（plugins）

> 分析日期：2026-10-05 ｜ 范围：前端 `src/features/plugins/**`、`src/lib/market/**`（9 文件 556 行）+ 后端 `src-tauri/src/market/**`（mod.rs 701 + registry.rs 230）
> 负责：主会话 / plugins-module
> 设计依据：`docs/08-插件清单与自动恢复设计.md`

## 0. 速览

| 项         | 值                                                                                                                                                 |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| 一句话定位 | DSH 插件市场：推荐目录（社区策展 tar.gz）+ npm 搜索 + 已装管理，前端负责浏览（走系统代理），Rust 只做「变更面」的唯一信任边界                      |
| 完成度     | **90/100（优秀）**                                                                                                                                 |
| 代码规模   | 前端 858 行（+ 32 行 `sync.test.ts`）/ 后端 931 行（**含 4 个 Rust 单测** ~86 行）                                                                 |
| 最严重问题 | 安装走 `pnpm add`，会执行包的 `postinstall` 脚本——`lifecycle_scripts` 已采集但**标记了 `#[allow(dead_code)]` 从未用于决策**（`registry.rs:27-28`） |

### 值得称道的部分

这是全项目**安全设计最扎实**的模块，值得作为其他模块的范式：

- **信任边界划分极清晰**：文件头注释就说清了「Rust 不掺和浏览，只保留变更面的信任边界——安装前在这里按请求的版本复核 registry 状态，**不信任前端传来的结论**」（`market/mod.rs:1-7`、`registry.rs:4-6`）。前端把 `compatibility` 算一遍只是展示用，装的时候 Rust 再算一遍。
- **「安装成功」的判定不看退出码**：`pnpm` 退出 0 不算数，去 `node_modules` 里查实际落盘版本（`mod.rs:141-159`）。而且不一致时**报错但保留现场、不自动回滚**（`mod.rs:142-143` 注释解释了为什么：让用户用卸载清理，比偷偷回滚更可解释）。
- **兼容门槛对 rc 钉版生态现实做了正确妥协**：`registry.rs:113-148` 同时试「原样」与「去预发布」两种形态，任一命中即放行；未声明/解析不了都放行——「门槛只拦确凿的不兼容」。这个取舍有 4 个 Rust 单测锁定（`registry.rs:220-246`）。
- **前端零依赖解包 tar.gz**：`tar.ts` 手写 tar reader + WebView2 原生 `DecompressionStream`（28 行），并**跟随 `dist.tarball` 而非自己拼 URL**（`catalog.ts:54-55` 注释：镜像会重写 host，跟着走才能留在镜像内）。
- **失败即错误，不静默降级**：`catalog.ts:5-6`「陈旧的目录是错误答案，不是降级答案」。

---

## 1. 模块定位与设计意图

`docs/08` 的 11 节设计在代码里的落点：

| 设计节                                | 落点                                                                      | 状态                            |
| ------------------------------------- | ------------------------------------------------------------------------- | ------------------------------- |
| §2 数据形状（清单 = 包名 + 精确版本） | `settings::PinnedPlugin` + `mod.rs:161-172`                               | ✅                              |
| §3 安装成功判定（G1）                 | `mod.rs:141-159` 反向校验                                                 | ✅ 超出设计（设计只要求退出码） |
| §4 自动补装（G3+G4）                  | `market_sync_pinned` → `SyncItemResult[]`                                 | ✅                              |
| §5 导出/导入（G5）                    | 走备份域（`backup_*`），插件清单计入                                      | ✅                              |
| §7 边界与守护                         | 三道：`claim_busy` 并发位 / `ensure_not_safe_mode` / `registry::validate` | ✅                              |
| §11.3-4 安全模式冻结变更              | `mod.rs:50-58` + 前端 `frozen` 派生（`PluginsPage.svelte:257`）           | ✅ 前后端双道                   |
| §11 三态 profile 恢复                 | `harness/recovery.rs` 侧                                                  | ✅（由 env 模块覆盖）           |

**与设计的偏差**：`docs/08` §9 建议「5 笔 commit」的切分已落地，但**安装前是否要拦 lifecycle scripts，设计文档里没有这一项**——这是实现层发现的真实缺口（见 §4-D1），设计文档应回补。

---

## 2. 功能清单与完成度

### 2.1 浏览与搜索

| #   | 功能                                 | 状态   | 证据(file:line)                         | 备注                                             |
| --- | ------------------------------------ | ------ | --------------------------------------- | ------------------------------------------------ |
| 1   | 推荐目录拉取（`dsh-plugin-catalog`） | 已完成 | `catalog.ts:44-61`                      | 跟随 registry，10min 内存 TTL                    |
| 2   | tar.gz 零依赖解包                    | 已完成 | `tar.ts:11-35`                          | `DecompressionStream` + 手写 tar reader          |
| 3   | 目录字段归一化 + 容错                | 已完成 | `catalog.ts:63-95`                      | `npm` 为空的条目直接过滤（二期才支持 GitHub 源） |
| 4   | 分类筛选                             | 已完成 | `PluginsPage.svelte:527-555`            | 分类来自目录包，非硬编码                         |
| 5   | 关键词本地过滤                       | 已完成 | `PluginsPage.svelte:298-311`            | name/description/publisher 三字段                |
| 6   | 4 种排序（下载/收录/星数/策展原序）  | 已完成 | `PluginsPage.svelte:312-325`            |                                                  |
| 7   | npm registry 搜索                    | 已完成 | `npm.ts` + `PluginsPage.svelte:128-143` | 空词立即（生态发现），输入防抖 320ms             |
| 8   | 搜索竞态防护                         | 已完成 | `PluginsPage.svelte:129,134`            | `searchSeq` 序号校验，过期响应丢弃               |
| 9   | 分页（PAGE_SIZE=100）                | 已完成 | `PluginsPage.svelte:36,494`             | 「加载更多」式                                   |
| 10  | 兼容性按需检查（4 并发）             | 已完成 | `PluginsPage.svelte:157-185`            | 只在筛选非「全部」时触发，零浪费请求             |
| 11  | 详情缓存 + key 失效                  | 已完成 | `PluginsPage.svelte:70,79-81`           | key 含 `registry\0pinned\0name`，切换自动失效    |
| 12  | 详情对话框（覆盖层）                 | 已完成 | `MarketDetailDialog.svelte`             | 列表零跳动                                       |
| 13  | 打开插件主页                         | 已完成 | `PluginsPage.svelte:279-291,440-448`    | `httpsLink()` 校验 http(s) 才给按钮              |

### 2.2 变更面（Rust 唯一信任边界）

| #   | 功能                              | 状态             | 证据(file:line)           | 备注                                            |
| --- | --------------------------------- | ---------------- | ------------------------- | ----------------------------------------------- |
| 14  | 已装清单（profile manifest 事实） | 已完成           | `mod.rs:75-82`            | dependencies × bundles × node_modules           |
| 15  | 安装精确版本                      | 已完成           | `mod.rs:85-182`           |                                                 |
| 16  | **安装前 registry 复核**          | 已完成           | `mod.rs:100-110`          | 按**请求版本**复核，不信前端                    |
| 17  | 弃用/不兼容硬拦截                 | 已完成           | `registry.rs:79-89`       |                                                 |
| 18  | 包名白名单校验                    | 已完成           | `registry.rs:171-189`     | 含 scope，字符集白名单                          |
| 19  | 落盘反向校验                      | 已完成           | `mod.rs:144-159`          | 退出码 0 不算数                                 |
| 20  | 内核包禁卸                        | 已完成           | `mod.rs:27,191-193`       | `@deepseek-ai/` 前缀                            |
| 21  | 并发互斥                          | 已完成           | `mod.rs:31-48`            | `AtomicBool` + `BusyGuard` RAII                 |
| 22  | 安全模式冻结                      | 已完成           | `mod.rs:50-58`            | 双端（Rust 拒 + 前端禁用按钮）                  |
| 23  | 卸载                              | 已完成           | `mod.rs:185-200+`         |                                                 |
| 24  | 清单补装（逐项、失败不中断）      | 已完成           | `market_sync_pinned`      | 设置页也可触发（`SettingsPage.svelte:309-326`） |
| 25  | 生命周期脚本检测                  | **采集但未使用** | `registry.rs:27-28,93-97` | `#[allow(dead_code)]`                           |
| 26  | 安装输出进 supervisor 日志        | 已完成           | `mod.rs:93-96` 等 8 处    | 底部跟一行最新日志当进度                        |
| 27  | 超时控制                          | 已完成           | `mod.rs:22-24`            | 空闲 120s / 总 10min / drain 3s                 |

---

## 3. 实现分析

### 3.1 架构与数据流

```
浏览面（前端 webview，走系统代理）
  推荐: registry → dsh-plugin-catalog/latest → dist.tarball → fetchBytes
        → DecompressionStream(gzip) → tar.ts 抽出 package/plugins.json
        → normalizeCatalog（过滤 npm 为空的条目）→ 内存 TTL 10min
  搜索: registry search 端点（去中心化兜底）
  兼容: fetchNpmDetail(name, pinnedDshVersion) → 4 并发 + SvelteMap 缓存

变更面（Rust，唯一信任边界）
  market_install(name, version)
    ├ claim_busy()            ← AtomicBool RAII，挡住并发
    ├ ensure_not_safe_mode()  ← 安全模式冻结
    ├ registry::detail()      ← 按请求版本复核，不信前端
    │   ├ is_package_name()   ← 包名字符白名单
    │   ├ /versions/{version} ← 不比对 dist-tags.latest（允许装旧版）
    │   └ compatibility()     ← rc 钉版宽松判定
    ├ registry::validate()    ← 弃用 / 不兼容 拒绝
    ├ ensure_build_allowlist()
    ├ pnpm add {name}@{version}  → 输出进 supervisor 日志
    ├ mutate_bundles()        ← 加入 bundles 数组（启动加载）
    ├ deployed_version() 反查  ← 退出码 0 不算数
    └ settings.plugins.pin()  ← 记入插件清单（失败只 warn 不回滚）
```

**设计精髓**：浏览面故意放在前端（`mod.rs:2-4` 注释：webview fetch 走系统代理、npmmirror 全端点带 CORS），这是**性能与兼容性**的选择；而信任边界用「Rust 在安装时重算一遍」来保证，而不是靠前端自觉。这是正确的分层——把不可信的部分放在影响最小的地方。

### 3.2 前后端契约

- 4 个 `market_*` 命令在 `contract.ts:93-96` 声明，受 `contract.test.ts` **命令级**门禁保护。
- ⚠️ `MarketInstalled` / `SyncItemResult` **未进** `RESPONSE_TYPES`（`contract.test.ts:92-107`），字段漂移不会被拦截。对比 intel 有 4 个类型进名单。
- `InstalledBundle`（`mod.rs:62-72`）的 `rename_all = "camelCase"` ✅，`spec` 字段用空串表达「内核包无依赖范围」而非 `Option`——是刻意的（前端 `PluginsPage.svelte:395` 直接 `cached?.version ?? listing.version`）。

### 3.3 性能与资源

- **兼容性检查的请求量控制得很好**：`dshFilter === 'all'` 时**零请求**（`PluginsPage.svelte:109-113`），`checkProgress`（`:370-375`）给用户「正在检查 n/m」反馈。避免了一进入页面就打 100 个 registry 请求。
- `inflight: SvelteSet` 防止同一 key 重复请求（`PluginsPage.svelte:71,159-162`）。
- 安装超时控制完备：空闲 120s（无输出即判定卡死）、总 10min、结束 drain 3s（`mod.rs:22-24`）。
- `registry::fetch_json` 预算 20s，注释说明「镜像可能在美国之外的慢链路上」（`registry.rs:14-15`）。

### 3.4 错误处理

- 每一步失败都 `supervisor.note(Stderr, "[插件] 安装失败：…")`（`mod.rs:103,108,113,126,134,150,157`）——**错误在环境页日志里一定看得到**，这是很好的可观测性。
- 区分「安装失败」（回滚不了也不回滚）与「清单未更新」（只 warn，不影响本次安装）（`mod.rs:160-172`）——降级优先级明确。
- 前端三类错误分区展示：`error`（变更）/ `catalogError`（推荐）/ `searchError`（搜索），互不覆盖（`PluginsPage.svelte:587-595`）。

---

## 4. 缺陷清单

| 级别   | 问题                                                                                                                                              | 位置                                                          | 影响                                                                                                                                                                                | 证据                                                                                      |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| **P1** | 安装会执行 npm 包的 `preinstall/install/postinstall/prepare` 脚本，但采集到的 `lifecycle_scripts` **标记 `#[allow(dead_code)]` 从未参与任何决策** | `registry.rs:27-28,93-97`                                     | 恶意/被入侵的插件包可在安装时执行任意代码。`ensure_build_allowlist` 只管构建脚本（`mod.rs:112`），不管 install 类脚本                                                               | 采集代码完整存在（`registry.rs:93-97` 四个脚本名），`Detail` 字段却 `#[allow(dead_code)]` |
| **P2** | `MarketInstalled` / `SyncItemResult` 未纳入字段级合同门禁                                                                                         | `contract.test.ts:92-107`                                     | 已装清单字段漂移不会被 CI 拦截                                                                                                                                                      | `contract.ts:938,950` 有 interface，但不在 `RESPONSE_TYPES` 名单                          |
| **P2** | `is_package_name` 允许 `~` 字符                                                                                                                   | `registry.rs:187`                                             | `matches!(c, '-' \| '_' \| '.' \| '~')` — `~` 不是 npm 包名的合法字符（npm 允许 `-` `_` `.` 和 scope 的 `@` `/`）。虽然拼到 URL 里 `%7E` 会被解码回 `~`，实际风险低，但白名单应收紧 | npm 官方包名规则不含 `~`                                                                  |
| **P2** | 目录包 `dsh-plugin-catalog` 的 tar.gz **在前端完整解到内存**                                                                                      | `tar.ts:12-13` `new Response(stream).arrayBuffer()`           | 目录包若变大（社区插件上千），会一次性占用等量内存。`DecompressionStream` 流式解压后仍整体 buffer                                                                                   | `tar.ts:13`                                                                               |
| **P2** | 安装进度只能看 supervisor 日志尾行                                                                                                                | `PluginsPage.svelte:378` `progressLine = harness.logs.at(-1)` | pnpm 进度是多行的，只显示最后一行；用户看不到「正在下载依赖 x/y」                                                                                                                   | 无进度条/无阶段化进度                                                                     |
| **P3** | 搜索 tab 的 `searchSort` 声明了 4 个值，但 `searchBase` 直接等于 `searchResults`，**未见排序实现**                                                | `PluginsPage.svelte:34,353`                                   | 选「更新时间/下载数/名称」可能无效果                                                                                                                                                | `searchBase = $derived(searchResults)` 无 `.sort()`，对比 `featuredBase` 有完整 switch    |
| **P3** | 卸载没有二次确认                                                                                                                                  | `PluginsPage.svelte:238-249`                                  | 安装有 `installBlocked` 预检，卸载是单击即执行（`mod.rs:196` 先改 bundles 再 pnpm rm）                                                                                              | 项目已有 `ConfirmDialog.svelte` 组件可复用                                                |
| **P3** | `working` 是单值字符串，同时只能有一个变更在途                                                                                                    | `PluginsPage.svelte:50`                                       | Rust 侧 `claim_busy` 也是全局单并发，一致。但 UI 上其他行的安装按钮会 `disabled`（`:454`），用户无法批量操作                                                                        | 与 Rust 约束一致，非 bug，是设计                                                          |
| **P3** | `searchResults` 无分页/上限                                                                                                                       | `PluginsPage.svelte:63`                                       | registry search 返回多少就渲染多少（`searchShown` 无 slice，而 `featuredShown` 也没有）                                                                                             | 对比 `PAGE_SIZE` 只作用于 `featuredLimit`                                                 |

---

## 5. 改善建议

| 优先级 | 建议                                                                                                            | 改动文件                                                                 | 收益                                                      | 成本 |
| ------ | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------------- | ---- |
| **高** | lifecycle scripts 要么拦（`--ignore-scripts` + 显式提示）、要么在详情对话框明示「该包安装时会执行 postinstall」 | `registry.rs`（去掉 `#[allow(dead_code)]`）+ `MarketDetailDialog.svelte` | 补上唯一真正的供应链攻击面；`Detail` 已采集好数据，改动小 | 低   |
| **高** | 补 `searchBase` 的排序实现（`searchSort` 四个值）                                                               | `PluginsPage.svelte:353`                                                 | 消除「选了排序没反应」的功能欺骗                          | 极低 |
| **中** | `MarketInstalled` / `SyncItemResult` 加入 `contract.test.ts` 的 `RESPONSE_TYPES`                                | `contract.test.ts:92`                                                    | 补字段漂移盲区                                            | 极低 |
| **中** | 卸载加 `ConfirmDialog` 二次确认                                                                                 | `PluginsPage.svelte:238`                                                 | 与项目现有交互规范一致                                    | 极低 |
| **中** | `is_package_name` 去掉 `~`                                                                                      | `registry.rs:187`                                                        | 白名单精确化                                              | 极低 |
| **中** | 安装进度改成阶段化（解析 pnpm 输出为「下载/链接/构建」阶段），而非日志尾行                                      | `PluginsPage.svelte:378`                                                 | 长安装有可见进度                                          | 中   |
| **低** | tar 读取改为流式按块跳过，不整体 buffer                                                                         | `tar.ts:12-13`                                                           | 目录包规模无上限                                          | 中   |
| **低** | 搜索结果加 `PAGE_SIZE` 上限                                                                                     | `PluginsPage.svelte:353`                                                 | 极端 query 防护                                           | 极低 |
| **低** | `docs/08` §7 补一节「lifecycle scripts 策略」，把实现层的发现回补设计                                           | `docs/08`                                                                | 设计文档与实现同步                                        | 极低 |

---

## 6. 完成度评估

| 维度           | 得分 | 依据                                                                                                                                                |
| -------------- | ---- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| 功能完整性     | 95   | 27 项功能，25 项完整，1 项采集未用，1 项部分（搜索排序）                                                                                            |
| 健壮性与安全性 | 82   | 三道守护 + 双端冻结 + 不信前端，**扣分项是 P1 的 lifecycle scripts 缺口**                                                                           |
| 性能与资源     | 88   | 按需检查 + inflight 去重 + 完整超时控制，扣分为 tar 全量 buffer                                                                                     |
| 测试覆盖       | 70   | Rust 4 个单测（`registry.rs:195-279`）覆盖版本提取/兼容判定/门槛/包名，前端 `sync.test.ts`(32) + `semver.ts`(155 行) 有实现但**未见配套 test 文件** |
| UX 完整度      | 86   | 三 tab + 分类 + 4 排序 + 4 兼容筛选 + 详情弹层，进度反馈弱，卸载无确认                                                                              |
| 可维护性       | 92   | 信任边界注释清晰，`docs/08` 与实现对应紧密                                                                                                          |

**总评 90/100**：安全设计是全项目标杆，「不信任前端结论 + 落盘反向校验 + 只拦确凿不兼容」三条都做对了且有单测锁定。**唯一实质缺口是 lifecycle scripts**——数据已经采集在手却标了 `#[allow(dead_code)]`，说明设计时意识到了但没做完。

---

## 7. 测试与验证

### 实际执行

| 命令                     | 结果                                                                                                                               |
| ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------- |
| `npx vitest run`（全量） | ✅ 27 文件全过，261 passed / 1 skipped（262 total），3.75s。本模块命中 `sync.test.ts`                                              |
| 代码走查                 | 完整读取 `PluginsPage.svelte`(1-600/791)、`catalog.ts`(95)、`tar.ts`(35)、`market/mod.rs`(1-200/701)、`market/registry.rs`(280 全) |

### 已有测试清单（全仓 157 个 Rust 单测中，market 占 10）

| 文件                           | 单测数 | 覆盖内容                                                                                                                                                             |
| ------------------------------ | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `market/mod.rs`                | 6      | 安装流程与 profile 清单                                                                                                                                              |
| `market/registry.rs`           | 4      | `版本提取按请求取_缺失给latest提示`（`:203`）/ `兼容判定对rc钉版宽松`（`:221`，4 种情形）/ `详情提取门槛字段且validate拦截`（`:249`）/ `包名校验覆盖scope`（`:273`） |
| 前端 `lib/market/sync.test.ts` | 32 行  | 同步结果摘要                                                                                                                                                         |

### 未执行 / 覆盖盲区（诚实标注）

- ❌ **未执行** `cargo test --manifest-path src-tauri/Cargo.toml`（上述 10 个 Rust 单测**未实际运行**，仅代码走查确认其存在与逻辑）
- ❌ **未执行** `pnpm exec svelte-check`
- ⚠️ **无端到端验证**：未实际安装/卸载任何插件，因此 `pnpm add` 的真实行为、`deployed_version` 反查、bundles 写入后的 DSH 启动加载**均未运行时验证**
- ⚠️ **无并发验证**：`claim_busy` 的互斥、`BusyGuard` 的 RAII 释放仅靠代码审查，未做并发压测
- ⚠️ **覆盖盲区**：`semver.ts`(155 行) 与 `npm.ts`(135 行) **无测试文件**；前端兼容性计算（`compatBadge`）、`tar.ts` 的 tar 解析、`catalog.ts` 的字段归一化均无测试
