# docs/14 — 每局全量复盘与 Laya 经验沉淀

> 状态：已实现（见 git log `feat(games)` 后续提交）。
> 上位文档:docs/11(实验室)、docs/12(双脑)、docs/13(进化闭环)。
> **本文修订 docs/13 §3.1 触发策略**:由「有新信息才调」升级为「每死必析」。

## 0. 动机

用户实战观察:死亡过程中存在大量**过程异常**——直接掉崖、直接撞小兵、
在同一地点反复停留。旧触发器(新死亡签名才调 Qwen)会把这些重复失败
去重丢弃,导致:

- 第 2..N 次同类死亡的过程信息(死前决策、门控抖动、停滞时长)从未被分析;
- 「反复循环很多次后碰巧过去」的低效路径,本应在下一轮就被优化掉;
- 复盘只盯着「怎么不死」,没人回答「**喂给 Laya 的数据是否合理**」——
  哪些观测该增、哪些该删、哪些参数在浪费 token 与延迟。

本项目的终极目的不是通关马里奥,而是**沉淀 Laya 的使用经验**,为 Laya
在更多领域的应用做探索。因此复盘必须做到两件事:

1. **每一局死亡都分析完整过程**(规则层全量 + Qwen 层全量);
2. 把「给 Laya 喂什么数据」本身纳入可进化的沙盒空间,让经验可沉淀、
   可消融、可泛化。

## 1. 不变式(沿用 docs/13 §1,不松动)

- 局内配置快照:`driver.beginRun(policy)` 冻结,热更新下一局才生效;
- 规则复盘先行,Qwen 慢思在后;
- Qwen 补丁是候选,必须过评审才 commit(LLM 负责创意,代码负责纪律);
- 本地 agent 只动沙盒策略空间,永不碰代码;改代码是云端代理的事;
- Qwen 分析全程异步,**绝不阻塞下一局**。

## 2. 触发策略升级:每死必析

`shouldInvokeQwen` 新语义(替代 docs/13 §3.1):

| 条件                   | 是否调 Qwen   | reason                             |
| ---------------------- | ------------- | ---------------------------------- |
| 同一 runId 已分析      | ❌            | `already-analyzed`(幂等)           |
| 通关                   | ✅            | `win`                              |
| 死亡(任何签名,含重复)  | ✅            | `death(sig)` / `death-repeat(sig)` |
| 无死亡无通关(中途停局) | plateau 时 ✅ | `plateau`                          |

变化点:**重复死亡签名不再去重**,只降级为 reason 标注(`death-repeat`),
写进提示词让 Qwen 知道「这是第 N 次死在同一地方」——恰恰是重点分析对象。

并发护栏:UI 单飞(`qwenRefining` 锁,重入直接丢弃并由下一局补上——
每局都触发,丢一局不等于丢信息);headless 顺序循环天然串行。

## 3. 过程轨迹与异常检测(规则层,每局必跑,零 LLM)

### 3.1 死亡上下文快照(driver)

driver 维护每 attempt 的决策 ring;死亡事件落盘时,把**死前最近 8 条决策**
(action/conf/gate/col)作为 `death-context` 事件行写入会话。直接回答:
「直接掉崖」前两拍打出的动作是什么、置信度多少、门控放行还是升级了。

### 3.2 新增异常指标(postmortem.ts,全部规则判定)

| 指标               | 判定规则                                 | 指向的异常                                             |
| ------------------ | ---------------------------------------- | ------------------------------------------------------ |
| `firstDeathTick`   | 首次死亡的 tick                          | 开局几十 tick 就死 = 直接掉崖模式                      |
| `thrashSites`      | 同 col(±2)内 EXECUTE↔ESCALATE 往返 ≥3 次 | 门控抖动:模型犹豫不决                                  |
| `churnSites`       | 同 col 连续 ≥3 拍相同动作且 maxX 无进展  | 原地空转(与 stallSites 互补:stall 看位置,churn 看决策) |
| `deaths[].context` | 死前决策序列(来自 3.1)                   | 死因的直接证据链                                       |

### 3.3 过程 digest(给 Qwen 的输入)

`processDigest(report)`:把全量复盘压成 ≤1.5k token 的紧凑摘要——
死亡+死前上下文、停滞站点、抖动/空转热区、时间线、门控分布。
**完整过程以 digest 形式给 Qwen**,原始 JSONL 仍在会话文件里供离线批扫。

## 4. 观测模式进化:把「喂什么数据」纳入沙盒

### 4.1 背景

Laya 请求体(brain.ts `encodeState`)由 5 个观测件组成:progress 行、
mario 位姿、profile16(前方 16 列首个实心行)、threats 命名威胁清单、
subgoal。哪些是必需、哪些在浪费 token、16 列是否最优——**这本身应该被进化**。

### 4.2 沙盒字段(policy.ts 新增,值域硬钳制)

| 字段                | 值域   | 默认 | 语义                         |
| ------------------- | ------ | ---- | ---------------------------- |
| `obsProfileCols`    | 8-24   | 16   | profile 前瞻列数             |
| `obsThreatsLookPx`  | 96-288 | 176  | 威胁清单前扫距离             |
| `obsIncludePose`    | bool   | true | 是否携带 vx/vy/onGround/face |
| `obsIncludeSubgoal` | bool   | true | reflex 模式是否携带 subgoal  |

「移除某数据」= 对应开关调 false 或窗口调小;「增加」= 反向。
全部走既有 sanitize → limitPatch → 冷却 → 候选评审管线,
**消融效果由 judgeCandidate 实测裁决**,不靠猜。

### 4.3 消融可追溯

history 行与 evolution.jsonl 审计行新增 `obs` 快照
(4 个观测字段值)。回放任意一局都能还原当时的观测模式。

## 5. Qwen 输出 schema 扩展

```json
{
  "playbook": "...",      // 战术手册(≤1600 字,滚动,人保段保留)
  "policy_patch": {...},  // 原有字段 + obs* 观测字段
  "insight": "..."        // 新增:一条可泛化的 Laya 使用经验(≤200 字)
}
```

- `insight` 聚焦**元层**:Laya 请求数据的合理性(哪个观测件有用吗/
  置信度有何变化)、参数增删理由、门控与延迟的关系——不写游戏战术。
- 每次分析必须基于过程 digest 指出**至少一个过程异常**及其处置
  (写进 playbook);`insight` 可空(空则不沉淀)。
- 提示词注入 `death-repeat` 标注:同一签名第 N 次死亡时,要求 Qwen
  聚焦「为什么优化没生效」而不是重复描述死法。

## 6. 经验沉淀:insights.md

新增存储文件 `mario/insights.md`(与 playbook 平级):

- 滚动上限 2400 字;**人保段机制与 playbook 相同**(`<!-- HUMAN -->`
  段永不被覆盖);
- 每条 insight 带迭代号与观测快照,形成时间序列;
- 定位:**跨游戏可泛化**的 Laya 使用经验库——未来 Laya 进新领域时,
  这份文档就是先验知识(观测设计、门控调参、延迟预算的实证结论)。

## 7. 成本与节奏

- 每局 1 次 Qwen 调用(本地 :17230,异步):一局 30s+,分析 ~5-10s,
  队列几乎不积压;UI 单飞丢弃由下一局补上,headless 串行无丢失。
- 规则层(3.1/3.2)零 LLM、零延迟,每局必跑——即使 Qwen 不在线,
  过程异常也已落盘进复盘文档。

## 8. 实施映射

| 模块                | 变更                                                                         |
| ------------------- | ---------------------------------------------------------------------------- |
| `policy.ts`         | 新增 4 个 obs 字段 + 值域钳制                                                |
| `brain.ts`          | `encodeState` 按 policy 裁剪观测件                                           |
| `driver.ts`         | 死亡上下文快照(死前 8 决策 → `death-context` 事件)                           |
| `postmortem.ts`     | firstDeathTick / thrashSites / churnSites / deaths[].context / processDigest |
| `evolution.ts`      | 触发器每死必析;audit 带 obs 快照                                             |
| `qwen.ts`           | 提示词扩展(过程 digest + obs 字段说明 + death-repeat);insight 解析           |
| `MarioLab.svelte`   | insights.md 持久化与面板展示;观测配置显示                                    |
| `e2e/mario-loop.ts` | headless 同步(每死必析 + insights)                                           |
| 测试                | 触发器/新指标/obs sanitize/insight 解析                                      |
