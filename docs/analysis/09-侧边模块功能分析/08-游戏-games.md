# 侧边模块分析报告 · 游戏（games）

> 分析日期：2026-10-05 ｜ 范围：`src/features/games/**` + `src-tauri/src/mario/**`、`src-tauri/src/tools/{laya,qwen,moli}.rs`
> 负责：worker / games-module
> 方法：独立一手分析（通读全部前端 + 后端源文件、跑 vitest）。已有评审 `docs/analysis/15-1~15-5` 的结论仅作交叉参考并逐条复核，标注为「已有评审亦指出」；复核结果写在 §4 各条「复核」栏。

## 0. 速览

| 项         | 值                                                                                                                                                           |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 一句话定位 | 全项目最大的自进化实验室：确定性马里奥引擎 + Laya(System 1)/ Qwen(System 2)双脑 + 每局复盘 + 补丁评估闭环，外加一个无进化能力的俄罗斯方块对照组              |
| 完成度     | **70/100（B−：主链路跑通，但存在 1 个 P0 显示缺陷与 4 个 P1 闭环失效点）**                                                                                   |
| 代码规模   | 前端 8763 行（生产 6770 / 测试 1993）／后端 1004 行（mario 134 + tools 870）／Rust 测试 0 行                                                                 |
| 最严重问题 | **切关卡后画布永久变黑**：`switchLevel` 置 `level=null` 声称「触发 $effect 重建背板」，但全组件没有任何 `$effect` 会重建它（`MarioLab.svelte:108`vs`:1294`） |

---

## 1. 模块定位与设计意图

`games`（侧栏第 8 项）不是一个「游戏合集」，而是 **Laya 决策引擎跨领域可用性的验证场**。设计脉络：

- **docs/09~11**：确立「只读探活、绝不代启 sidecar」的进程纪律（游戏页不碰 `laya_start`/`qwen_start`），以及「确定性固定步长引擎 + 置信度门控三分支」的核心范式。
- **docs/12**：双脑架构 —— Laya 管逐拍 System 1，Qwen(:23661) 管局间 System 2 监督。
- **docs/13/14/15/16**：把「AI 在玩」升级为「AI 在自我进化」—— 策略沙箱（policy）、每局全量复盘、Laya 经验沉淀、消息内容设计变体、进化对话台。

两个实验室定位不同：

|            | Laya Jump（马里奥）                        | Laya Reflex（俄罗斯方块）                                 |
| ---------- | ------------------------------------------ | --------------------------------------------------------- |
| 角色       | 进化闭环的**主战场**                       | System 1 反射的**对照组/教学场**                          |
| System 2   | rollout 搜索（planner，docs/16 §2）        | Dellacherie 2-block lookahead 求解器                      |
| 进化能力   | 完整（Qwen 提案 → 评估 → commit/rollback） | **无**（只有 Laya + 求解器，无 Qwen、无复盘、无策略沙箱） |
| 设计成熟度 | 高（v3，纪律层完备）                       | 中（v1，未接入 v3 体系）                                  |

这个分工是合理的：进化复杂度集中在单一实验室，避免两处都做半套。但**方块模块完全没有 v3 演进**（无 policy 沙盒、无洞察沉淀、无指标），意味着 docs/15 的方法论目前只在 1/2 的游戏上被验证。

---

## 2. 功能清单与完成度

### 2.1 马里奥实验室

| #   | 功能                                                 | 状态         | 证据(file:line)                                    | 备注                                                                                                                                                                                                                                                                                                                   |
| --- | ---------------------------------------------------- | ------------ | -------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1  | 确定性固定步长引擎 `step()`                          | 已完成       | `mario/engine.ts:361`                              | 60Hz、纯函数、零时钟读取；`stateHash` 逐 tick 锁定（`engine.test.ts` 确定性用例通过）                                                                                                                                                                                                                                  |
| M2  | 关卡数据（1-1 / 1-2）                                | 已完成       | `mario/world1-1.ts:128`,`:233`,`:351`              | 声明式生成 `Uint8Array`，零二进制资源；坑宽 ≤ 助跑跳距的不变量有单测                                                                                                                                                                                                                                                   |
| M3  | 变身 / 受伤 / 无敌窗                                 | 已完成       | `engine.ts:616`,`:627`                             | fire→big→small→死；受击有 `invuln`                                                                                                                                                                                                                                                                                     |
| M4  | 龟壳三态（walk/shell/slide）                         | 已完成       | `engine.ts:821`,`:855`                             | 含滑壳连锁清场、复活倒计时                                                                                                                                                                                                                                                                                             |
| M5  | 食人花                                               | 已完成       | `engine.ts:644`                                    | 贴近管道时不伸出（经典规避）                                                                                                                                                                                                                                                                                           |
| M6  | 道具（火球 / 蘑菇 / 花 / 吐币）                      | 已完成       | `engine.ts:427`,`:557`,`:958`                      | 火球触地弹跳，否则 30 tick 埋进地里                                                                                                                                                                                                                                                                                    |
| M7  | 通关序列（旗杆→城堡→消失）                           | 已完成       | `engine.ts:1058`,`:1073`                           | 含时间奖励结算                                                                                                                                                                                                                                                                                                         |
| M8  | 死亡重生                                             | **有缺陷**   | `engine.ts:1038-1054`                              | `respawn` **不重置** `tiles` / `blocks` / `coins`。第 2 命起世界已被破坏（碎砖仍在、?块已 used、金币已 taken），与真实 SMB 行为相反，且使同局多命不可比 → 见 D6                                                                                                                                                        |
| M9  | 离屏渲染（3.4MB 背板 + 每帧 blit）                   | 已完成       | `mario/renderer.ts:26`,`:145`                      | `image-rendering: pixelated`；实体逐个视口裁剪                                                                                                                                                                                                                                                                         |
| M10 | HUD / 叠加层（意图框 / S2 得分条 / 涟漪 / 输入量子） | 已完成       | `renderer.ts:329`,`:338`,`:356`,`:365`             | HUD 走 DOM 覆盖层保证任意 DPI 锐利（docs/15 §1）                                                                                                                                                                                                                                                                       |
| M11 | 主循环（rAF + 固定步长累加器 + 速度滑杆）            | 已完成       | `MarioLab.svelte:568-659`                          | `document.hidden` 时不累加（`:572`）；单帧最多 8 步防雪崩                                                                                                                                                                                                                                                              |
| M12 | 生命周期清理（rAF / setInterval / ResizeObserver）   | 已完成       | `MarioLab.svelte:1351-1356`, `:553`                | 三者全部清理，**未发现泄漏**                                                                                                                                                                                                                                                                                           |
| M13 | 切关卡                                               | **有缺陷**   | `MarioLab.svelte:105-112`,`:628`,`:1294`           | **P0**：置 `level=null` 后无任何 `$effect` 重建 → 画布永久黑屏。见 D1                                                                                                                                                                                                                                                  |
| M14 | 键盘（人玩）                                         | 已完成       | `MarioLab.svelte:479-524`                          | ←→/AD/Z/空格/X/J/Shift；P 暂停、Enter 重生、Esc 关窗                                                                                                                                                                                                                                                                   |
| M15 | AI 大脑 brain（v3-obs 编码）                         | 已完成       | `mario/brain.ts:125`,`:201`                        | RLE 压 profile（省 60%+ token）、`encoding: 'v3-obs'` 常态 ≤160 字符（有单测锁定）                                                                                                                                                                                                                                     |
| M16 | 置信度门控三分支                                     | 已完成       | `brain.ts:34`,`:274`                               | EXECUTE/RE_SENSE(≤2 次防抖)/ESCALATE；集外 choice 按概率回退候选集                                                                                                                                                                                                                                                     |
| M17 | 观测沙盒消融（几何 + 消息设计）                      | 已完成       | `mario/policy.ts:12-52`,`:174`                     | 枚举白名单 + 白名单清单 + 整数量化；越界写 issue 可观测                                                                                                                                                                                                                                                                |
| M18 | 规划器 planner（rollout 搜索 System 2）              | 已完成       | `mario/planner.ts:236`,`:287`,`:342`               | 纯函数、WeakMap 缓存 (state,tick)、搜索池随形态裁剪（fire_right 仅 fire 态）                                                                                                                                                                                                                                           |
| M19 | 规划器可达性自检                                     | 已完成       | `planner.ts:353` + `planner.test.ts`               | `canClearGap` 与 `JUMP` 表同源推导并锁定                                                                                                                                                                                                                                                                               |
| M20 | 驾驶舱 driver（节流 / in-flight≤1 / attempt 守卫）   | 已完成       | `mario/driver.ts:362`,`:414`                       | stale 决策丢弃并记账；`sessionSnapshot` 浅拷贝防队列引用活对象                                                                                                                                                                                                                                                         |
| M21 | 物理护栏（跳跃弧补全 / 停滞自愈 / 本能否决）         | 已完成       | `driver.ts:269-347`                                | 三条护栏均有单测（`driver.test.ts` 3 例）                                                                                                                                                                                                                                                                              |
| M22 | 局内策略快照（不可变��变量①）                        | **有缺陷**   | `driver.ts:196`,`:203` vs `:368`,`:442`            | `beginRun` 快照正确，但停滞兜底与 ESCALATE 机动调 `autopilotInput(s, world)` **漏传策略**，回退全局 → 见 D7                                                                                                                                                                                                            |
| M23 | Qwen 慢思监督（prompt / 解析 / 边界）                | 已完成       | `mario/qwen.ts:75`,`:296`,`:359`                   | 顶层键白名单 + 枚举注册表 + 值域钳制 + `deathKey` 幻觉校验；超时 60s                                                                                                                                                                                                                                                   |
| M24 | 死亡级全量载荷（即死即析）                           | 已完成       | `qwen.ts:169`,`:406`                               | 分层裁剪：PROTECTED ③④ 保底、TRIMMABLE ⑤→②→①                                                                                                                                                                                                                                                                           |
| M25 | 每局复盘 postmortem                                  | 已完成       | `mario/postmortem.ts:135`,`:291`,`:389`            | 规则判定零 LLM；含 thrash/churn/firstDeathTick 等 8 类异常热区                                                                                                                                                                                                                                                         |
| M26 | 复盘地标归属                                         | **有缺陷**   | `postmortem.ts:103`                                | 硬编码 `WORLD_1_1`；玩 1-2 时管道/坑/台阶地标全错 → 污染 Qwen 归因。见 D10                                                                                                                                                                                                                                             |
| M27 | 进化闭环 evolution（状态簿记 / 触发器 / 补丁纪律）   | 已完成       | `mario/evolution.ts:411`,`:575`,`:622`,`:727`      | 单维度强制 + ±20% 步进钳制 + 冷却期 + 滞回判定，均有单测                                                                                                                                                                                                                                                               |
| M28 | 审计日志（evolution.jsonl，只追加）                  | 已完成       | `evolution.ts:829`,`:833`                          | 版本白名单不静默丢行                                                                                                                                                                                                                                                                                                   |
| M29 | Laya 经验沉淀 insights.md                            | 已完成       | `evolution.ts:914`,`:882`                          | 人保段永不丢；Jaccard ≥0.55 折叠为 `‖确认@iter`；4800 字上限                                                                                                                                                                                                                                                           |
| M30 | 版本账本 / 回归守卫                                  | 已完成       | `evolution.ts:975`,`:1002`,`:1025`                 | cap 20；连续退化自动回退最优版本                                                                                                                                                                                                                                                                                       |
| M31 | 分析调度器（合并 / cap / 优先级 / 熔断）             | **有缺陷**   | `mario/machine.ts:92`,`:128`                       | `scheduleTask`/`nextTask`/`CircuitBreaker` 本身正确且有单测，但**未接线**：UI 的溢出任务被丢弃（见 D3）、熔断无恢复路径（见 D4）                                                                                                                                                                                       |
| M32 | 显式状态机 `transition()`                            | **占位骨架** | `machine.ts:38-69`                                 | 9 状态表实现完整、**有单测**，但 **UI 从未调用**（`MarioLab.svelte:76-83` 只 import 了 `CircuitBreaker/nextTask/removeTask/scheduleTask/taskFailed`）。实际闭环状态由 8 个散落布尔量（`qwenRefining`/`analysisPending`/`stepGated`/`trialArmed`/`runRecorded`/`runHadTasks`/`runAllOk`/`breakerTripped`）承担 → 见 D12 |
| M33 | 进化对话台 EvolutionChat                             | 已完成       | `mario/EvolutionChat.svelte`, `mario/chat.ts`      | 5 元素消息流、占位→结论原地变身、审计降级重建、锚定+未读计数                                                                                                                                                                                                                                                           |
| M34 | 即死即析接线                                         | 已完成       | `MarioLab.svelte:371-401`                          | 死亡瞬间入队，不等重生                                                                                                                                                                                                                                                                                                 |
| M35 | 分析硬门（上一局没析完不开新局）                     | **有缺陷**   | `MarioLab.svelte:675-684`,`:1277-1280`             | 最坏可阻塞开始按钮约 12 分钟且无取消 → 见 D8                                                                                                                                                                                                                                                                           |
| M36 | 三态干预（自动/单步/暂停）                           | 已完成       | `MarioLab.svelte:1211`,`:1255`                     | 单步挂起 + 「装填下局 ▸」放行                                                                                                                                                                                                                                                                                          |
| M37 | 逐局全量留存 runs/                                   | 已完成       | `MarioLab.svelte:255-302`                          | 超 1 MiB 自动降级紧凑版；索引行 + G6 修正行                                                                                                                                                                                                                                                                            |
| M38 | 存储层（Tauri IPC + localStorage 兜底）              | 已完成       | `mario/store.ts:19`,`:57`                          | 隐私模式降级为内存语义不阻断游戏                                                                                                                                                                                                                                                                                       |
| M39 | 后端状态读写（白名单 / 原子写 / 只追加）             | 已完成       | `src-tauri/src/mario/commands.rs:66`,`:100`,`:113` | 路径白名单 `[A-Za-z0-9._-]` ≤128 + 最多一层子目录；无穿越面                                                                                                                                                                                                                                                            |
| M40 | 侧栏 Laya 进程（启停/探活/收割）                     | 完成但有缺陷 | `src-tauri/src/tools/laya.rs:196`,`:327`,`:352`    | **红线遵守良好**：只杀自有子进程、外部实例仅提示；`spawn_blocking` 不卡 UI；但有并发双启动竞态（见 D13）与 PID 复用风险（见 D14）                                                                                                                                                                                      |
| M41 | 侧栏 Qwen 进程（同上）                               | 完成但有缺陷 | `src-tauri/src/tools/qwen.rs:193`,`:311`,`:340`    | 整树 `taskkill /T /F`（server.py 会拉引擎子进程，只杀父进程留孤儿）—— 设计正确；同样有双启动竞态                                                                                                                                                                                                                       |
| M42 | Laya 探活（只读）                                    | 已完成       | `MarioLab.svelte:1338-1347`                        | 10s 轮询；**未代启**（符合 docs/11 §1.1）                                                                                                                                                                                                                                                                              |
| M43 | 会话 JSONL 落盘（sidecar）                           | 完成但有缺陷 | `driver.ts:72-115`                                 | 不可达时静默降级；但 `flush` 失败回插队首**无上限**，sidecar 长期不可达时 `pending` 数组无界增长                                                                                                                                                                                                                       |
| M44 | 恢复：策略 / 手册 / 簿记 / 候选 / 聊天               | 完成但有缺陷 | `MarioLab.svelte:1298-1337`                        | 功能齐全；但审计/聊天文件**全量读入**（见 D11），且异步 IIFE 无卸载取消（见 D15）                                                                                                                                                                                                                                      |
| M45 | 终局评分与候选判定                                   | **部分完成** | `MarioLab.svelte:762-888`                          | commit/rollback 逻辑正确，但：**单局判定**，`DESIGN_EVAL_RUNS=5` 被 `void window` 丢弃（见 D9）；无基线时**无条件直接转正**（`:768-813`），未做任何测量                                                                                                                                                                |

### 2.2 俄罗斯方块实验室

| #   | 功能                                          | 状态         | 证据(file:line)                             | 备注                                                                                                                           |
| --- | --------------------------------------------- | ------------ | ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| T1  | 引擎（碰撞 / 消行 / 特征 / 7-bag）            | 已完成       | `tetris/engine.ts:223`,`:271`,`:313`,`:508` | 8 条单测全通过                                                                                                                 |
| T2  | Dellacherie 2-block 求解器                    | 已完成       | `engine.ts:430`,`:463`                      | 有专门单测验证「贪心会造死缝、lookahead 不会」                                                                                 |
| T3  | Laya 反射 brain（hex 编码 / 约束解码 / 门控） | 已完成       | `tetris/brain.ts:101`,`:129`,`:236`         | hex 编码把 210 字符点阵压到 20×4 hex；reflex 下候选集收敛到 2~3 个                                                             |
| T4  | 四种驱动模式                                  | 已完成       | `TetrisLab.svelte:815`                      | reflex / pure / solver / human                                                                                                 |
| T5  | 竞态防线（pieceGen）                          | 已完成       | `TetrisLab.svelte:191`,`:334`               | 方块切换后旧决策作废（继承 mario 的教训）                                                                                      |
| T6  | 重力 / 决策双定时器 + 生命周期清理            | 已完成       | `TetrisLab.svelte:392-410`,`:495-501`       | `clearInterval` 在 onMount cleanup 里                                                                                          |
| T7  | 三栏 SENSE/DECIDE/ACT + 目标框                | 已完成       | `TetrisLab.svelte:640`,`:555`               | fuchsia 虚线目标与 mario 语义统一                                                                                              |
| T8  | 侧栏日志（JSONL session）                     | 完成但有缺陷 | `TetrisLab.svelte:86-132`                   | 同 M43：`pendingLogs` 无上限回插                                                                                               |
| T9  | 错误提示                                      | 已完成       | `TetrisLab.svelte:632`                      | laya 不可达时给出手动启动命令                                                                                                  |
| T10 | **进化闭环（Qwen / 复盘 / 策略沙盒）**        | **未实现**   | 全目录无相关文件                            | 方块实验室完全不在 docs/13~16 的进化体系内 —— 设计上可接受（对照组定位），但需在文档中显式声明，否则读者会误以为它也有进化能力 |

---

## 3. 实现分析

### 3.1 引擎与渲染

**质量最高的子系统。** `engine.ts` 的四条自律（无随机源、无时钟读取、状态不进响应式、瓦片按局拷贝）让它同时满足「可单测 + 可回放 + 可批量生成蒸馏数据」三个用途 —— 这是 Laya 跨领域实验的前提，架构判断正确。

两个观察：

1. **确定性是被测试锁死的，不是口头承诺**。`engine.test.ts` 有「同一输入脚本两次运行逐 tick 指纹一致」「不同 viewW 下小怪激活完全一致」两条 —— 后者尤其重要，说明 `viewW` 参数没有意外进入物理。`stateHash`（`engine.ts:1123`）为此专门存在。
2. **物理常量对三形态一致**（`engine.ts:12-13`）是有意的简化，让 `JUMP` 表对 small/big/fire 同时有效，避免规划器可达性推理分叉。这是「为可分析性牺牲一点真实性」的合理取舍，且在注释里写明了。

渲染侧 `renderer.ts` 的「一次性绘入 3584×240 离屏 canvas，每帧只 blit 视口」是正确选择：避免了每帧遍历 224×15 = 3360 个瓦片。HUD 走 DOM 覆盖层而非画进 canvas，也是对「任意 DPI 锐利」的合理取舍。

### 3.2 AI 决策链路（brain/policy/planner/qwen/双脑）

链路是清晰的四层：

```
GameState ──profile16/rle──> v3-obs state（≤160 字符）
                                │
                    ┌───────────┴───────────┐
              System 1 (Laya)          System 2 (rollout 搜索)
              brain.decide()            planner.nextIntent()
                    │                           │
              门控三分支 ◄──── 候选集约束 ◄────┘
                    │
              driver.inputFor() ──> Input 量子 ──> step()
                    │
              [四条物理护栏兜底]
```

几个做得好的地方：

- **候选集约束解码**（`planner.ts:318` → `brain.ts:276`）。Laya 在 >20 选项时急剧退化，所以搜索把候选压到 Top-3（`searchTopK` 可调 2~5），且 Laya 选到集外时按概率回退 —— 这是把不可靠组件约束到「怎么选都不会太坏」的典型手法。
- **搜索纯函数 + WeakMap 缓存**（`planner.ts:281`）。autopilot 每 tick 调一次、叠加层再调一次，缓存把重复成本降到零（`planner.test.ts` 有「同 tick 缓存命中」用例）。
- **Qwen 输出边界靠解析器硬隔离**（`qwen.ts:314-347`）。顶层键白名单、静态性声明表（`policy.ts:29-52`）、值域钳制、deathKey 幻觉校验 —— 不依赖 prompt 自觉。`system` prompt 里还专门声明「你是局间策略优化者，不是逐拍玩家」，防止模型自我升级权限。这是本模块最值得肯定的设计。

**但护栏层有一个真实漏洞**（D7）：

```ts
driver.ts:306   this.held = autopilotInput(s, world, P);        // ✅ 传了快照策略
driver.ts:368   this.held = autopilotInput(s, world);           // ❌ 漏传
driver.ts:442   this.held = d.escalateManeuver ? autopilotInput(s, world) : ...  // ❌ 漏传
```

`autopilotInput` 第三参为 `undefined` 时走 `getPolicy()`（全局 champion），而非 `beginRun` 冻结的 `runPolicy`。这**直接违反 docs/13 不变量①「局内配置不可变」**：停滞兜底拍与 ESCALATE 机动拍使用的是「当前全局策略」而非「本局快照策略」。一旦 Qwen 提案在局中被 commit（或用户在设置面板拖了滑杆），同一局内前后两种拍就在用不同策略 —— 而复盘归因假设它们是同一个。已有评审 `15-4` 曾指出「回调时刻读 getPolicy() 有陈旧风险」，复核**仍成立且已具化为具体漏参**。

### 3.3 进化闭环与复盘

设计上最成熟的一块。「LLM 负责创意，代码负责纪律」这个分工落实得很彻底：

| 纪律                               | 代码位置                  | 是否强制           |
| ---------------------------------- | ------------------------- | ------------------ |
| 局内配置不可变                     | `driver.ts:196`           | ✅（但见 D7 漏参） |
| 规则复盘在前（触发器只吃 report）  | `evolution.ts:411`        | ✅                 |
| 补丁是候选（测量通过才提交）       | `evolution.ts:575`        | ✅                 |
| 单维度变更（数值与消息设计不同车） | `evolution.ts:666`        | ✅                 |
| 步进限幅 ±20% / 窗口 ±8px          | `evolution.ts:697`,`:712` | ✅                 |
| 字段冷却期                         | `evolution.ts:727`        | ✅                 |
| 成功才占幂等键                     | `evolution.ts:456`        | ✅                 |
| 分析失败有界（≤2 次后放弃）        | `machine.ts:122`          | ✅                 |
| 不丢死亡                           | `machine.ts:88-91` 注释   | ❌ 见 D3           |

**但闭环有三处会「静默失效」**（D3/D4/D9），共同特征是：代码路径存在且有单测，但**接线不完整**，导致单元测试全绿而线上不工作。这是本模块最需要警惕的失效模式 —— 绿灯并不代表闭环通。

### 3.4 侧栏进程生命周期与 IPC 契约

**后端质量明显高于前端**，且严格遵守了项目红线：

- `laya.rs:113` / `qwen.rs:110` 探活用 `TcpStream::connect_timeout` 显式超时，注释写明「系统栈默认 SYN 重传会拖到秒级，同步命令跑在主线程时表现为整页周期性卡顿（实测教训）」—— 这是真实的踩坑记录，不是防御性编程。
- `laya_start` / `qwen_start` / `*_stop` 全部 `spawn_blocking`，绝不进 async runtime 阻塞。
- `running` 以健康探测为准而非 PID 判断，外部手动启动的实例同样识别（`laya.rs:208`）；`stop` 只处理 state 里记录的 PID，外部实例返回「请在原处停止」（`laya.rs:340`）—— **与 AGENTS.md §2 红线完全一致**。
- `qwen.rs:311` 的 `kill_tree` 用 `taskkill /T /F` 而非 `child.kill()`，因为 server.py 会再拉起引擎子进程 —— 注释说明了原因，这是正确的领域知识。
- 应用退出统一收割（`lib.rs:389-390`）。

两处缺陷见 D13/D14。

契约侧：`contract.ts:75-91` 声明了 `mario_state_*` 与 `laya/qwen/moli` 全部命令，`store.ts:59-66` 统一走 `call()` 包装，无裸 `invoke` —— 符合 docs/03 §7「合同唯一」。

### 3.5 性能与内存

**没有发现真实性能问题**，逐项核查：

- **rAF 泄漏**：无。`MarioLab.svelte:1352` 正确 `cancelAnimationFrame`。
- **`setInterval` 未清理**：无。探活与 flush 两个定时器都在 cleanup 里清（`:1353-1354`）。
- **页面隐藏后仍跑**：无。`:572` 显式判 `!document.hidden`。
- **ResizeObserver**：`:553` 返回 `disconnect()`。
- **对象无界增长**：`chat` cap 199（`chat.ts:53`）、`sentLog` cap 24（`driver.ts:413`）、`log` cap 40（`MarioLab.svelte:212`）、`insights` 4800 字、`versions`/`ledger` cap 20、`history` cap 200 —— **全部有上限**。
- **每帧重建对象**：无。`buildLevelCanvas` 只在 mount 时调一次（`:1294`）。
- **rollout 搜索成本**：O(11 动作 × 45 horizon) ≈ 500 次 `step()`/tick，被 WeakMap 缓存削到每 tick 一次。`engine.test.ts` 的「autopilot 全程通关」用 982ms 跑完整局（数千 tick），即约 0.14ms/tick；速度 3× + 8 步/帧的上限下约 1.1ms/帧，仍远低于 16.6ms 预算。
- **Laya 分析阻塞 UI 线程**：`refineViaQwen` 是 async fetch，不阻塞 rAF。但 **`buildDeathPayload` / `buildDeathPayloadFromSent` 是同步字符串组装**（`qwen.ts:169`,`:406`），在 `onDeathAnalyze` 里被调用，而后者由 `frame()` 的事件循环直接触发（`MarioLab.svelte:613`）。载荷上限 24000 字符，实测量级在 1ms 内，当前不构成问题，但它是主线程上唯一随数据量线性增长的路径。

**唯一实际的内存隐患**是 M43 的 `pending` 数组与 T8 的 `pendingLogs`：sidecar 不可达时 `flush` 每 5s 尝试一次、失败把 batch 回插队首，而 `push` 仍在持续追加。10s 探活失败只置 `layaReady=false`（停止**决策**），不停止**日志推送** —— 所以「游戏照常但日志堆积」的组合是可达的。

---

## 4. 缺陷清单

| 级别         | 问题                                         | 位置                                                                                                                                                     | 影响                                                                                                                                                                                                                                                                                                                                                                                                                                                 | 证据                                                                                                                |
| ------------ | -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| **D1 / P0**  | 切关卡后画布永久黑屏                         | `mario/MarioLab.svelte:108`                                                                                                                              | `switchLevel` 置 `level = null` 并注释「触发 $effect 重建背板」，但组件内**只有 2 处 `$effect`**（`:538` ResizeObserver、`:1294`onMount），**没有任何`$effect`会重建`level`**。`buildLevelCanvas`全文件仅在`:1294` 被调用一次。`:628`的`if (canvasEl && ctx && level)`此后永假 →`drawFrame`再不执行。1-1→1-2 切换后画面冻结在旧帧，HUD 却显示`WORLD 1-2`，状态与画面长期不一致                                                                       | `MarioLab.svelte:108`, `:628`, `:1294`；`Select-String '$effect'` 仅命中 2 处                                       |
| **D2 / P1**  | Laya `/predict` 无超时 → 决策永久停摆        | `mario/brain.ts:332-343`                                                                                                                                 | `ask()` 的 `fetch` 不带 `AbortSignal`（对比同文件 `:353` 的 `checkEndpoint` 有 1500ms 超时、对比 `qwen.ts:386` 有 60s 超时）。若 sidecar accept 后不响应，`maybeDecide` 的 `await` 永不 settle → `inFlight` 永为 `true`（`driver.ts:388` 置位，仅 `:488` finally 复位）→ 整个 run 内 `inputFor` 永远拿不到新动作，游戏静止。**10s 探活无法自救**（探活只改 `layaReady`，不改 `inFlight`）                                                            | `brain.ts:336-342`, `:353`;`driver.ts:387-388`, `:488`                                                              |
| **D3 / P1**  | 队列溢出的死亡任务被静默丢弃                 | `MarioLab.svelte:341-344` + `machine.ts:107`                                                                                                             | `scheduleTask` 返回的 `overflow` 在 UI 侧**只被计数**（`bumpQwen(evo,'droppedByCap')`）后丢弃，直接违背 `machine.ts:90` 注释「溢出弹出最低优先级任务交由宿主落盘 backlog(**不丢死亡**)」与 docs/15 §6.2 的核心承诺。同时 `evo.pendingAnalyses`（`evolution.ts:224`）**UI 从不写入**（仅 `e2e/mario-loop.ts:738` 写），而 `planDeathTask`/`planAnalyses` 的幂等去重又依赖它（`evolution.ts:395`,`:417`）—— 该字段在 UI 路径上恒为空，去重检查是死代码 | `MarioLab.svelte:344`;`machine.ts:88-91`,`:107`;`evolution.ts:224`,`:395`,`:417`;`grep pendingAnalyses` 仅 e2e 写入 |
| **D4 / P1**  | 熔断器无恢复路径 → 进化永久死亡              | `MarioLab.svelte:1033-1036`,`:1342-1347` + `machine.ts:127`                                                                                              | `machine.ts:127` 注释声明「探活成功由宿主调 reset 复位」，但 `MarioLab.svelte:1342` 的探活定时器**只更新 `layaOk/qwenOk`，从不调 `breaker.ok()`**；`breaker.ok()` 唯一调用点是 `:1104` 的分析成功分支。而 `:1033` 在循环开头 `if (breaker.tripped) break`。⇒ 一旦连续失败 3 次熔断，**再也无法恢复，直到重启应用**。UI 提示「Qwen 熔断,探活恢复自启」（`EvolutionChat.svelte:106`）是**错误承诺**                                                    | `machine.ts:127-139`;`MarioLab.svelte:1033`,`:1104`,`:1342-1347`;`EvolutionChat.svelte:106`                         |
| **D5 / P1**  | 队列在 Qwen 恢复后无人排空                   | `MarioLab.svelte:358`,`:971-974`                                                                                                                         | `kickQueue` 首行 `if (qwenRefining \|\| !qwenOk) return`。Qwen 离线期间 `onDeathAnalyze`（`:372`）与局末 `kickQueue`（`:971`）双双早退，任务滞留在 `taskQueue`。恢复后**没有任何路径重新触发 `kickQueue`**（只在 `onDeathAnalyze` 与 `genPostmortem` 内调用）。⇒ 若此后不再死亡/通关，已排队的分析无限期滞留；`analysisGate` 亦不会建立，硬门形同虚设                                                                                                | `MarioLab.svelte:358`, `:372`, `:971-974`                                                                           |
| **D6 / P1**  | 重生不重置世界 → 同局多命不可比              | `mario/engine.ts:1038-1054`                                                                                                                              | `respawn` 重置 mario/敌人/花/道具/计时/相机，但**不重置** `s.tiles`、`s.blocks`、`s.coins`、也不清 `s.deaths`/`s.maxX`。后果：第 2 命起碎砖已碎、?块已 used、金币已 taken、分数沿用 —— 与真实 SMB「死亡重置关卡」相反。更关键的是复盘与评分：同一 run 内不同 attempt 的 `maxX` 被并入同一个 `runScore`，而**世界难度在 attempt 间已改变**，使 `judgeCandidate`（`evolution.ts:575`）的中位分失去可比性                                               | `engine.ts:1038-1054`;`MarioLab.svelte:765`;`evolution.ts:575`                                                      |
| **D7 / P1**  | 兜底拍漏传局内策略快照 → 违反不可变��变量①   | `mario/driver.ts:368`, `:442` vs `:306`                                                                                                                  | `autopilotInput(s, world)` 省略第三参 → `planner.ts:326` 回落 `getPolicy()` 全局策略。停滞兜底拍（12 拍）与 ESCALATE 机动拍因此使用「当前 champion」而非 `beginRun` 冻结的 `runPolicy`。一旦局中策略变更（Qwen 提案转正或用户拖滑杆），同局内两批拍用不同策略，复盘归因失效。已有评审 `15-4` 亦指出回调时点读 `getPolicy()` 的陈旧风险，**复核仍成立并已定位到具体行**                                                                               | `driver.ts:306`, `:368`, `:442`;`planner.ts:326`                                                                    |
| **D8 / P2**  | 分析硬门可阻塞开始按钮约 12 分钟且无取消     | `MarioLab.svelte:675-684`, `:1277-1280`                                                                                                                  | `await analysisGate` 无超时。队列上限 `QUEUE_CAP=6` + backlog 无上限，每任务最多 2 次尝试（`machine.ts:122`），单次超时 60s（`qwen.ts:20`）。且 `:972-973` 在 Qwen 离线时把 `remaining` **重复 push 回 `taskQueue`**（`enqueueTasks` 已在 `:948` 加过一次），进一步放大队列                                                                                                                                                                          | `MarioLab.svelte:675-684`, `:948`, `:972-973`;`machine.ts:84`,`:122`;`qwen.ts:20`                                   |
| **D9 / P2**  | 消息设计补丁被单局判定                       | `MarioLab.svelte:766-767`                                                                                                                                | `const window = c.kind === 'design' ? DESIGN_EVAL_RUNS : 1; void window;` —— 明确计算 5 局评估窗后**丢弃**，design 补丁按单局判定。docs/15 §6.4 的理由是「输入漂移需更长观测」，单局噪声完全可吞掉设计变更的真实效果，导致误 commit / 误 rollback。`evalMedian`（`evolution.ts:565`）在 UI 路径上从未被调用                                                                                                                                          | `MarioLab.svelte:766-767`;`evolution.ts:53`,`:565`;`grep evalMedian` 仅 evolution + e2e                             |
| **D10 / P2** | 复盘地标硬编码 1-1，1-2 数据全错             | `mario/postmortem.ts:103`                                                                                                                                | `const WORLD = WORLD_1_1;` 被 `landmarkOf()`（`:106-119`）使用。玩 1-2 时管道/坑/台阶地标按 1-1 布局匹配 → 失败地点系统性误标 → 直接污染喂给 Qwen 的归因素材与 `deathSignature`（`evolution.ts:375`）→ 进化信号失真。`MarioLab` 已支持切关（`:105`），故该路径可达                                                                                                                                                                                   | `postmortem.ts:103`, `:106-119`;`evolution.ts:375`;`world1-1.ts:233`                                                |
| **D11 / P2** | 审计/聊天日志全量读入内存                    | `MarioLab.svelte:1316`, `:1331-1335`                                                                                                                     | mount 时 `store.read(EVOLUTION_FILE)` 读取**整个** append-only 审计日志并逐行 `JSON.parse`，只为取最后 10 条（`.slice(-10)`）。聊天路径更糟：`:1335` 在 `chat.jsonl` 缺失时**再读一遍** evolution.jsonl 走 `rebuildFromAudit`。两个文件都无磁盘轮转（`evolution.jsonl` 只追加、`chat.jsonl` 只追加）。随运行月数线性增长，成为启动延迟与内存的持续来源                                                                                               | `MarioLab.svelte:1316-1318`, `:1331-1335`;`evolution.ts:829`                                                        |
| **D12 / P2** | 显式状态机实现完整但从未接线                 | `mario/machine.ts:38-69` vs `MarioLab.svelte:76-83`                                                                                                      | `transition()` + 9 状态表 + `LoopState` 是 docs/15 §3.1 的规定实现，单测覆盖主链与非法迁移，但 UI 只 import 了 `CircuitBreaker`/`nextTask`/`removeTask`/`scheduleTask`/`taskFailed`。实际状态由 8 个散落布尔量承担 ⇒ docs/15 §3.1「统一行为，不抽状态机则第二次分歧」的目标未达成，且这 8 个布尔量之间的一致性只能靠阅读 1300 行组件来保证。已有评审 `15-4` 亦指出 UI 与 e2e 各写一套隐式状态机，**复核仍成立**                                      | `machine.ts:38-69`;`MarioLab.svelte:76-83`;`grep transition(` 仅 machine + evolution.test                           |
| **D13 / P1** | 侧栏进程并发双启动竞态 → 孤儿进程            | `laya.rs:208-216`,`:263-275`;`qwen.rs:205-211`,`:252-264`                                                                                                | `laya_start_blocking` 先 `probe_health` 判「未运行」，再 `spawn`，最后把 child 无条件写入 `state.child`（`:272-275`）。两次并发调用（用户连点）**都能通过 running 检查并各自 spawn** → 第二个覆盖 state，第一个 child 的 handle 丢失 ⇒ 该进程永不被 `kill_owned`（`:352`）收割，成为真正的僵尸。Qwen 同构，且其就绪窗口长达 300s（`qwen.rs:30`），窗口期双启概率显著更高                                                                             | `laya.rs:208`,`:263-275`,`:352`;`qwen.rs:30`,`:205`,`:252-264`                                                      |
| **D14 / P2** | `taskkill /PID` 存在 PID 复用误杀风险        | `qwen.rs:311-326`                                                                                                                                        | `kill_tree` 用 `child.id()` 取 PID 交给 `taskkill /T /F /PID`，**未先 `try_wait()` 确认进程已退出**。Windows 在进程终止后不保留 PID，该 PID 可能已被复用 ⇒ 可能终止无关进程树。这触及 AGENTS.md §2「绝不能 kill 用户进程」红线，当前概率低但后果严重。`laya.rs:336` 的 `child.kill()` 走 handle，相对安全                                                                                                                                            | `qwen.rs:311-326`;`laya.rs:336`                                                                                     |
| **D15 / P3** | onMount 异步加载无卸载取消                   | `MarioLab.svelte:1298-1337`                                                                                                                              | IIFE 内多次 `await store.read(...)` 后写 `$state`，无 abort/销毁检查。用户在加载期间返回游戏列表 → 组件已卸载仍在写状态并做解析工作                                                                                                                                                                                                                                                                                                                  | `MarioLab.svelte:1298-1337`                                                                                         |
| **D16 / P3** | 日志 `pending` 数组无界增长                  | `mario/driver.ts:92-112`;`TetrisLab.svelte:110-132`                                                                                                      | sidecar 不可达时 `flush` 失败把 batch 回插队首（`driver.ts:108`），无上限无淘汰                                                                                                                                                                                                                                                                                                                                                                      | `driver.ts:108`;`TetrisLab.svelte:128`                                                                              |
| **D17 / P3** | 列表 key 可能碰撞                            | `MarioLab.svelte:1695`;`TetrisLab.svelte:757`                                                                                                            | `(row.t + row.text)` / `(entry.t + entry.text)` 作 key。`t` 精确到毫秒，同毫秒内相同文本（尤其 repeat 决策的固定文案）会产生重复 key，Svelte keyed each 抛错                                                                                                                                                                                                                                                                                         | `MarioLab.svelte:1695`;`TetrisLab.svelte:757`                                                                       |
| **D18 / P3** | 复盘 score 取 5s 陈旧快照                    | `mario/driver.ts:327-336` + `postmortem.ts:262`                                                                                                          | `final` 仅每 300 tick（5s）更新，复盘 `score` 来自该快照而非终局值。慢局影响小，但快死局会显示过时分数                                                                                                                                                                                                                                                                                                                                               | `driver.ts:327-336`;`postmortem.ts:262`                                                                             |
| **D19 / P3** | `history.iteration` 用数组长度而非簿记迭代号 | `MarioLab.svelte:753-754`                                                                                                                                | `iteration: runHistory.length + 1` 与 `evo.iteration`（`evolution.ts:531`，受 `recordRun` 独立维护）在历史截断/重置后失同步，`insights` 的 `- [iter N]` 标签会错位                                                                                                                                                                                                                                                                                   | `MarioLab.svelte:753-754`;`evolution.ts:531`                                                                        |
| **D20 / P3** | EvolutionChat 存在未清理的 setTimeout        | `EvolutionChat.svelte:126`                                                                                                                               | 展开时 `setTimeout(scrollToBottom, 0)` 未保存/未在卸载时清除                                                                                                                                                                                                                                                                                                                                                                                         | `EvolutionChat.svelte:126`                                                                                          |
| **D21 / P3** | 状态机/存储/日志等子系统零单测               | `mario/machine.ts`（接线部分）、`mario/store.ts`、`mario/driver.ts:72-115`、`mario/renderer.ts`、`src-tauri/src/mario/commands.rs`、`laya.rs`、`qwen.rs` | 1993 行测试全在纯函数侧（engine/policy/planner/brain/qwen/postmortem/evolution/chat/tetris-engine）。**闭环编排与 IO 边界完全无测** —— 这正是 D3/D4/D5/D6/D13 能长期存活的原因：它们的正确性无法由现有测试网捕获                                                                                                                                                                                                                                     | 各文件；见 §7                                                                                                       |

### 复核历史评审结论的结果

| 已有评审结论（`docs/analysis/15-*`）        | 本次复核                                                                                    |
| ------------------------------------------- | ------------------------------------------------------------------------------------------- |
| 15-4「UI 与 e2e 各写一套隐式状态机」        | **仍成立**，且更精确：`machine.ts` 的显式状态机已实现并有单测，但 UI 未接线（D12）          |
| 15-4「回调时刻读 `getPolicy()` 有陈旧风险」 | **仍成立**，已定位为 `driver.ts:368`/`:442` 漏参（D7）                                      |
| 15-2「进化质量依赖真机跑批」                | **仍成立**：`evalMedian`/中位评估仅存在于 e2e，UI 走单局（D9）                              |
| 15-5「UI 文案与可观测性」                   | **部分失效**：`EvolutionChat.svelte:106` 的「探活恢复自启」是错误承诺（D4），属可观测性误导 |
| 15-1「可靠性」                              | **部分成立**：进程红线遵守良好，但并发双启（D13）与无超时（D2）是新发现                     |

---

## 5. 改善建议

按 收益/成本 排序。

| 优先级 | 建议                                                                                                                                                                                                            | 改动文件                                                                   | 收益                                                                                            | 成本                                        |
| ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------- |
| **P0** | 让 `level` 真正重建：把 `switchLevel` 的 `level = null` 改为新增 `$effect(() => { if (level === null) level = buildLevelCanvas(world); })`，或直接在 `switchLevel` 内 `level = buildLevelCanvas(getWorld(key))` | `MarioLab.svelte:105-112`                                                  | 修复黑屏 P0；同时消除「注释承诺了不存在的机制」这一误导                                         | 极小（3 行）                                |
| **P0** | 给 `brain.ask()` 加超时：`AbortSignal.timeout(3000)`（与 `intervalMs` 同量级），并在 `driver.maybeDecide` 的 catch 里同时复位 `inFlight`                                                                        | `brain.ts:336`, `driver.ts:480-489`                                        | 消除「决策永久停摆且探活无法自救」的死局；sidecar 半死不再锁死整局                              | 极小（2 行），有 `fetchImpl` 注入点便于测试 |
| **P1** | 熔断恢复接线：探活定时器里 `if (driver.qwenReady) breaker.ok();`，并在 UI 文案里区分「熔断中(待探活恢复)」与「离线」                                                                                            | `MarioLab.svelte:1342-1347`, `EvolutionChat.svelte:96-118`                 | 消除进化永久死亡；修正错误承诺                                                                  | 极小（3 行）                                |
| **P1** | 溢出任务落盘：`enqueueTasks` 里把 `r.overflow` 写入 `evo.pendingAnalyses` 并 `persistEvo()`；mount 时用 `evo.pendingAnalyses` 初始化 `taskBacklog`                                                              | `MarioLab.svelte:339-349`, `:1314-1316`                                    | 兑现「不丢死亡」承诺；让 `planDeathTask` 的去重检查从死代码变活逻辑                             | 小（~15 行）                                |
| **P1** | 队列排空触发：把 `kickQueue` 从 `onDeathAnalyze`/`genPostmortem` 两处调用改为在探活定时器里也调用一次（`qwenOk` 由 false 转 true 时）                                                                           | `MarioLab.svelte:1342-1347`                                                | 消除 Qwen 恢复后队列无人消费的滞留；顺带让硬门恢复意义                                          | 小（~5 行）                                 |
| **P1** | 补齐策略快照漏参：`driver.ts:368`/`:442` 改用 `autopilotInput(s, world, this.policy())`                                                                                                                         | `driver.ts:368`, `:442`                                                    | 恢复 docs/13 不变量①；复盘归因可信                                                              | 极小（2 行）                                |
| **P1** | 并发双启防护：`laya_start_blocking`/`qwen_start_blocking` 进入后先取一把独立的 `starting: Mutex<bool>`（或复用 child mutex 做「非空即视为启动中」检查），启动中直接返回当前快照                                 | `laya.rs:205-208`, `qwen.rs:202-205`                                       | 消除孤儿进程；配合 `kill_owned` 保障退出收割完整                                                | 小（各 ~8 行）                              |
| **P1** | PID 复用防护：`kill_tree` 前先 `child.try_wait()`，已退出则跳过 taskkill                                                                                                                                        | `qwen.rs:311-326`                                                          | 收紧「绝不能 kill 用户进程」红线                                                                | 极小（3 行）                                |
| **P2** | `respawn` 重置世界：补 `s.tiles = world.tiles.slice(); s.blocks = <重建>; s.coins = d.coins.map(...)`，并按设计决定 `score/coinCount` 是否沿用                                                                  | `engine.ts:1038-1054`                                                      | 同局多命可比，`judgeCandidate` 中位分恢复意义；行为对齐真实 SMB                                 | 小（~8 行）                                 |
| **P2** | 地标按世界注入：把 `postmortem.ts:103` 的模块级 `WORLD` 改为 `SessionInput` 上携带 `worldKey`，`landmarkOf(world, col)` 按关卡取表                                                                              | `postmortem.ts:103-119`, `MarioLab.svelte:748-758`                         | 1-2 的失败归因正确；进化信号不再被系统性误标                                                    | 中（~25 行，含调用点与单测）                |
| **P2** | 分析硬门加超时与取消：给 `await analysisGate` 套 `Promise.race([gate, timeout(180s)])`，并在工具条提供「跳过等待」按钮                                                                                          | `MarioLab.svelte:675-684`, `:1277-1280`                                    | 消除最长 12 分钟的不可中断等待                                                                  | 小（~20 行）                                |
| **P2** | design 补丁走多局评估：`void window` 改为真正的 N 局缓冲（候选 + 计数，达标才判），或至少在 UI 明确标注「design 变更按单局判定，信噪比低」                                                                      | `MarioLab.svelte:766-767`, `:762-888`                                      | 减少设计变更误判                                                                                | 大（需引入评估窗状态机，~80 行）            |
| **P2** | 日志尾部读取：Rust 侧加 `mario_state_tail(name, n)` 命令只回末 N 行；或前端加行数上限截断                                                                                                                       | `src-tauri/src/mario/commands.rs`, `store.ts`, `MarioLab.svelte:1316-1335` | 启动开销与内存不再随运行月数增长                                                                | 中（前后端各 ~30 行）                       |
| **P2** | 显式状态机接线：把 `phase` 收进 `machine.ts` 的 `transition()`，UI 只发 event；或明确记录「`transition()` 为 e2e/文档保留」并从 UI 侧删除误用预期                                                               | `MarioLab.svelte:76-83`, `machine.ts`                                      | 消除文档-实现分叉（当前最危险的认知陷阱：代码在、测试在、但没用）                               | 大（重构 ~200 行）                          |
| **P3** | 文件拆分：`MarioLab.svelte`（1828 行）应拆为 `mario/panels/*`（顶栏/舞台/浮层）、`mario/useEvolutionLoop.ts`（闭环编排）、`mario/useRunLoop.ts`（rAF 主循环）                                                   | `MarioLab.svelte`                                                          | 触及 docs/03 §2.3「Svelte 组件 300 行」上限（现超 6.1 倍）；使 D3~D9 这类接线缺陷有独立可测单元 | 大（~1 人日）                               |
| **P3** | `pending` 队列加上限（如 200 条后丢弃最旧并计数）                                                                                                                                                               | `driver.ts:92-112`, `TetrisLab.svelte:110-132`                             | 消除唯一的无界增长路径                                                                          | 极小                                        |
| **P3** | 关键路径补测：`machine.ts` 接线、`store.ts` 降级、`driver.ts` session logger、`commands.rs` 的 `resolve()` 路径白名单（尤其 `runs/../` 拒绝）、`laya.rs`/`qwen.rs` 的双启竞态                                   | 新增 `*.test.ts` + Rust `#[cfg(test)]`                                     | 让 P1 类缺陷在 CI 可捕获                                                                        | 中                                          |

**关于 docs/03 编码规范的符合性**：

| 条款                               | 现状                                                                                                                                                                               |
| ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §2.3 Svelte 组件 / ts 模块 ≤300 行 | **严重超限**：15 个文件超限，最严重 `MarioLab.svelte` 1828 行（6.1×）、`engine.ts` 1040、`evolution.ts` 947、`driver.ts` 524。规范自称「上限是烟雾报警器」，此处已长期鸣响而未处理 |
| §5「组件不做业务」                 | **违反**：`MarioLab.svelte` 承载了完整的进化闭环编排（约 600 行业务逻辑），与 §5「数据获取和业务规则在 `api.ts`/`store.svelte.ts`」冲突                                            |
| §5「副作用必须可清理」             | ✅ 符合（rAF/interval/observer 全部清理，见 §3.5）                                                                                                                                 |
| §7「合同唯一」                     | ✅ 符合（`contract.ts:75-91` + `store.ts` 统一 `call()`）                                                                                                                          |
| §6「禁止 unwrap/expect」           | ✅ 符合（Rust 侧无裸 unwrap）                                                                                                                                                      |
| §4 注释规范                        | ✅ **优秀**：模块头齐全、注释解释「为什么」而非复述参数（例：`brain.ts:264-265` 解释 fetch 必须解绑 this 否则 WebView2 报 Illegal invocation —— 这是真实的踩坑知识）               |

**结论：注释与边界纪律是本模块的强项，文件尺寸与「组件不做业务」是明确失分项。**

---

## 6. 完成度评估

**总分：70 / 100（评级 B−）**

| 维度             | 分     | 依据                                                                                                                                                                                                                                                                          |
| ---------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 功能完整性       | **82** | 两个实验室的引擎/渲染/输入/UI 全部完整可用（`engine.ts`、`renderer.ts` 质量高）；`engine.ts`/`world1-1.ts`/`postmortem.ts`/`evolution.ts` 的功能面覆盖 docs/11~16 的绝大部分条目。扣分：切关卡失效（D1）、状态机未接线（D12）、方块无进化（T10，属定位选择）                  |
| AI 闭环完整性    | **74** | 闭环骨架、纪律层、边界隔离、可证伪 insight 都做到了工程级（`qwen.ts`/`policy.ts`/`evolution.ts`）。但**三条会静默失效的路径**：溢出不落盘（D3）、熔断不可恢复（D4）、队列恢复后无人消费（D5）；且 design 补丁单局判定（D9）、无基线无条件转正（M45）。修正后可达 88           |
| 健壮性与进程管理 | **58** | 后端进程纪律优秀：红线遵守（只杀自有）、`spawn_blocking`、整树 kill、外部实例识别、启动超时提示 —— 这些是真实踩坑沉淀。扣分：并发双启孤儿（D13）、PID 复用风险（D14）、Laya 无超时（D2）、熔断无恢复（D4）、pending 无界（D16）                                               |
| 性能与帧率       | **88** | 逐项核查无真实问题：rAF 无泄漏、隐藏页不跑、对象全部有 cap、rollout 有缓存且实测 ~0.14ms/tick、背板一次性绘制。扣分：主线程上同步组装 24k 字符载荷（`qwen.ts:169`，当前无害但随数据量线性增长）、`record.decisions` 单局内无 cap                                              |
| 测试覆盖         | **58** | 131 条测试 / 1993 行，**纯函数侧覆盖优秀**（确定性、门控、沙盒、补丁纪律、载荷裁剪、经验去重均有专门用例）。但**闭环编排与 IO 边界零覆盖**（D21）—— 这直接解释了为何 D3/D4/D5/D6/D13 这类缺陷能在全绿状态下长期存活                                                           |
| UX               | **80** | 剧院式舞台、HUD DOM 覆盖保证 DPI 锐利、进化对话台 5 元素消息流、三态干预、暂停/重生/重开齐备。扣分：D1 黑屏是硬伤、D8 最长 12 分钟无取消等待、设置面板滑杆不写回 policy（`MarioLab.svelte:1745` 仅 `onchange` 调 driver，不 `setPolicy`+`persistPolicy`，下次开局被快照覆盖） |
| 可维护性         | **52** | 注释与边界纪律是强项（模块头齐全、解释「为什么」、IPC 合同唯一、状态不进响应式）。但 `MarioLab.svelte` 1828 行承载全部闭环编排，8 个散落布尔量承担本该由状态机承担的一致性职责（`machine.ts` 状态机就在旁边却未接线）—— 违反 docs/03 §2.3 与 §5                               |

**加权理由**：功能与性能是真实高分（82/88），但健壮性、测试、可维护性三项均因同一根因（**接线层缺乏测试与抽象边界**）被系统性拉低。这不是「代码写得差」，而是「纪律建在了纯函数层，没有延伸到编排层」。

---

## 7. 测试与验证

### 实际执行

**命令 1** —— 前端单测（games 模块全量）：

```
> npx vitest run src/features/games
```

**结果：通过。**

```
 Test Files  10 passed (10)
      Tests  131 passed (131)
   Start at  04:14:35
   Duration  2.22s (transform 904ms, setup 0ms, import 1.27s, tests 2.11s, environment 1ms)
```

**命令 2** —— 逐用例清单（`--reporter=verbose`），用于确认覆盖边界：

```
> npx vitest run src/features/games --reporter=verbose
```

**结果：131/131 通过。** 覆盖分布（据输出归纳）：

| 测试文件                        | 覆盖内容                                                                                                                                                  |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `engine.test.ts`（26 例）       | 确定性指纹、视口无关性、跳跃高度/距离、?块/砖块/蘑菇/火球/变身/受伤/龟壳三态/食人花/旗杆通关/时间耗尽、**autopilot 全程通关 1-1 与 1-2**（982ms / 945ms） |
| `evolution.test.ts`（38 例）    | policy 信封迁移、触发器清单与幂等、中位分与滞回判定、限幅/单维度/冷却、审计往返、insights 滚动与人保段、经验去重、调度器与状态机、版本账本与回归守卫      |
| `planner.test.ts`（17 例）      | 危险感知、rollout 纯函数性与仿真隔离、候选集裁剪、autopilot 缓存命中、动作映射、跳跃可达性                                                                |
| `postmortem.test.ts`（9 例）    | 地标命名、统计口径、7 类规则假设、Markdown 小节、digest                                                                                                   |
| `qwen.test.ts`（5 例）          | 提示词构造、v3 输出解析、deathKey 幻觉拒绝、非 JSON 降级、playbook 截断                                                                                   |
| `brain.test.ts`（7 例）         | 门控三分支、v3-obs 编码与消融、**state ≤160 字符**、集外概率回退、RE_SENSE、ESCALATE                                                                      |
| `driver.test.ts`（3 例）        | 跳跃弧补全、human 模式不补全、停滞 6s 兜底                                                                                                                |
| `policy.test.ts`（6 例）        | 值域钳制、窗口逆序、ESCALATE≤EXECUTE 不变式、观测沙箱                                                                                                     |
| `chat.test.ts`（5 例）          | id 生成、滚动窗、原地变身、分组、坏行容错、审计降级重建                                                                                                   |
| `tetris/engine.test.ts`（8 例） | 碰撞、消行、特征、求解器、2-block lookahead、7-bag                                                                                                        |

### 未执行项及原因

| 项                                                    | 原因                                                                                                                                                                                                                                                                                                                                                                                                 |
| ----------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo test --manifest-path src-tauri/Cargo.toml`     | **未执行**。已先确认 `src-tauri/src/mario/*.rs` 与 `tools/{laya,qwen}.rs` 中 **`#[cfg(test)]` 数量为 0**（`Select-String` 验证），即这些文件当前没有任何 Rust 单测可跑；全量 `cargo test` 需编译整个 workspace（含 fff-core 等 crate），耗时远超本次分析窗口，且对本模块无额外信息量。`tools/moli.rs` 确有 4 条 Rust 单测，但属 R001 工具检测、与 games 模块无关                                     |
| 真机联调（启动 laya-server / Qwen、跑完整进化闭环）   | **未执行**。需外部 sidecar（`laya-server.exe` + `models/laya-multilingual-onnx` bundle，以及 Strata 程序目录 + 33-39GB 模型），不在本次环境内。所有关于运行时行为的判断均来自源码静态分析，已在文中逐条标注 `file:line`                                                                                                                                                                              |
| `pnpm e2e` / Playwright                               | **未执行**。需先 `pnpm e2e:build`（完整 Tauri debug 构建）。另注：`e2e/` 下与 mario/laya 相关的 9 个文件中，**只有 `probe-*.ts`（CDP 探针）与 `mario-loop*.ts`（无头闭环），无 `*.spec.ts`** —— `Get-ChildItem e2e -Filter *.spec.ts` 仅返回 `standalone-window.spec.ts`。即 mario/laya **没有 Playwright 端到端断言**，进化闭环的唯一回归保障是 `mario-loop.test.ts`（3.7KB，4 条左右）与纯函数单测 |
| `pnpm check` / `svelte-check` / `eslint` / `prettier` | **未执行**。本次职责为只读分析（仅允许写本报告文件），格式/lint 检查不改变任何结论。但需说明：`MarioLab.svelte` 中 `settings.speed` 等滑杆使用 `bind:value` 配 `min="0.5" step="0.1"` 却无 `max` 对应的数值绑定类型收窄，且 `:1358` `dot`/`pctOf` 等辅助函数定义在 `<script>` 尾部，`svelte-check` 的严格模式下这类模式通常需要确认类型推断 —— **此项未经工具验证，不作结论**                        |

### 验证结论

- ✅ **纯函数层可信**：131 条测试全绿，且覆盖了确定性、可达性、纪律边界等高价值不变量。`engine.test.ts` 的两条「autopilot 全程通关」是本模块最有价值的测试 —— 它证明 System 2 搜索在两关布局上都有效。
- ⚠️ **接线层不可信**：D1/D3/D4/D5/D6/D7/D13 全部位于「纯函数已验证、但组合方式未被测试」的接线层。这不是推测 —— `machine.ts` 的状态机有完整单测却从未被 UI 调用，正是这一失效模式的直接实例。
- 📌 **Rust 侧零单测**：`commands.rs` 的 `resolve()` 是本模块唯一的安全边界（路径穿越防护），当前无任何测试覆盖 `runs/../`、`.`、`超长名` 等拒绝路径。

---

## 附：一句话给决策者

这个模块的**架构判断和纪律设计是全项目最好的**（确定性引擎、纯函数纪律层、LLM 输出边界硬隔离、进程红线遵守），但它的**缺陷集中在最后一公里**：1300 行的 Svelte 组件承担了全部闭环编排，状态机建好却没接线，于是「测试全绿」与「闭环能跑」之间存在系统性缺口。建议按 P0→P1 的六条小改动（合计 <60 行）先恢复正确性，再考虑 P2/P3 的拆分与接线重构。
