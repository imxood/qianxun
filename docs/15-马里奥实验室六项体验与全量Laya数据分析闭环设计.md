# docs/15 · 马里奥自进化实验室 v3 设计方案(定稿)

> 状态:**设计定稿,未实施**。本稿合并 5 份独立评审(docs/analysis/15-1 ~ 15-5:
> 可靠性 / 进化质量 / Laya 消息设计 / 架构健全性 / UI 文案),取代本文档 v1 草案。
> 反馈六点:①canvas 文本模糊 ②统计 ③全自动候选 ④反射/纯反射 ⑤统一可拖动弹框
> ⑥**每死全量 Laya 数据分析**(重心)。
> 总准则:这是一套**自我进化**系统——健全(状态不能错)、可靠(证据不能丢、失败必须可见)、
> 不断进化(可归因、可收敛、可度量、可回退)。

---

## §0 设计宪法

1. **架构红线**:本地 Laya+Qwen 只在沙箱策略空间演化,永不改代码;云端模型只做代码修改/重编译,不进运行环路。
2. **权限矩阵**(完整表见 analysis-15-4 §权限矩阵):本地 agent 对一切持久状态只有**提案权与追加权**,裁决与改写永远在确定性代码;快脑零持久写、慢脑不进实时路径;**一切越权尝试必须可观测**(审计 issue,不许静默纠正)。
3. **文案原则(全局)**:一切文本——UI 标签、日志、面板说明、Laya 消息、insight——**简洁不罗嗦**。硬指标:标签 ≤6 字,日志行 ≤30 字,说明 ≤2 句;JSONL/审计保留原始枚举,UI 层查表转短语,两层文案不互渗。
4. **快慢脑边界靠解析器硬隔离,不靠模型自觉**:顶层键白名单 + 值域钳制 + 静态性声明(§6.6)。

---

## §1 canvas 文本模糊 → HUD 分层渲染

- 根因:HUD 8px 小字画进 240px 世界像素背板,CSS 放大 2.5~4× + DPI 缩放必糊;`pixelated` 对文本是锯齿放大器。
- 方案:游戏层(canvas,pixelated)只留关卡/实体/意图框/涟漪;**HUD 迁出 canvas 改 DOM 覆盖层**(font-mono 11px + 轻投影):SCORE/COINS/WORLD/TIME、IN 输入芯片、**门控徽标**(gate/conf/延迟,每拍更新)、阵亡/通关徽标。文本仅变化时写 DOM。
- `renderer.drawFrame` 删 HUD 段,留 `hudInCanvas?: boolean` 供无头截图对照。

---

## §2 统计数据

**计数器(state.json `counters`,唯一维护点 evolution.ts;审计为原始事实,启动时 counters == replay(evolution.jsonl) 对账)**:

```jsonc
{
  "runs": 0,
  "wins": 0,
  "deaths": 0,
  "deathsByCause": {},
  "deathsByLandmark": {},
  "qwen": {
    "deathAnalyses": 0,
    "runAnalyses": 0,
    "failed": 0,
    "timeouts": 0,
    "badJson": 0,
    "overLength": 0,
    "retries": 0,
    "merged": 0,
    "queueHighWater": 0,
    "emptyInsight": 0,
    "patches": 0,
    "commits": 0,
    "rollbacks": 0,
  },
  "perMode": { "reflex": { "runs": 0, "wins": 0 }, "pure": { "runs": 0, "wins": 0 } },
}
```

(v1 草案的 `bestScore` 删除——与 championScore 语义重叠;`avgLatencyMs` 走审计统计。)

**展示(evo 面板内嵌,不设独立 stats 窗)**:

```
┌ 进化 ──────────────────── [自动|单步|暂停] ┐
│ #12 · champion 1042 · 最远 87 列           │ ← 状态行
│ 本轮:试用 gapWindow[−16,16] · 待判定      │ ← 当前候选摘要
│ 消息:威胁named·进度full·提示full·默认     │ ← 消息设计选型一行
├─ 统计 ─────────────────────────────────────┤
│ 失败 38 ▲3   通关 6/44   分析 51(空 4)    │ ← Δ=近10局−前10局滑窗
│ 漏斗  提案18 ─▶ 转正5 ─▶ 回滚13(冷却2)    │ ← 有效学习率
│ 胜率  反射32% · 纯反射8%(人玩不计)        │
│ 队列 1 · 上次分析 3.4s                     │
├─ 学到了什么 ────────────────────────────────┤
│ ✦ 前扫288px 低置信决策减半        #10     │ ← 最近3条,单行≤24字
│ ✦ 维持:提示terse 省token不掉分   #12     │ ← "维持"行=emptyInsight 可见化
├─ 时间线(只读)─────────────────────────────┤
│ #12 ⚙ 试用#r17 → 转正 1042(+6%)           │
└─────────────────────────────────────────────┘
```

- `emptyInsight` 落点:统计格 `空 n` + 时间线「维持」行;死点 Top5 折叠;队列深度在此行。
- 补丁漏斗 = 系统有效学习率;胜率按模式分列(§4)。

---

## §3 候选全自动 + 统一状态机

### 3.1 最大架构债先还:两套隐式状态机已分叉

UI(单局判定/无基线人工处置)与 e2e(K 局中位/无基线直转)是两份隐式实现;v3 再写第三份分析队列必然失控。**抽 `machine.ts` 纯 reducer + `AnalysisScheduler` 纯逻辑,UI/e2e 复用同一份**(9 状态完整表——IDLE/TRAINING/POSTMORTEM/TRIGGER/QUEUED/ANALYZING/CANDIDATE_EVAL/VERDICT/PAUSED,含故障转移与崩溃恢复——见 analysis-15-4)。

### 3.2 交互:三态开关替代人工处置

| 态         | 行为                                                                                                   |
| ---------- | ------------------------------------------------------------------------------------------------------ |
| 自动(默认) | 提案→自动装填→自动判定→冷却,全程无人                                                                   |
| 单步       | 分析照跑,提案生成后挂起;「下一步 ▸」在下局 start() 时装填放行(沿用 armTrial 语义,避免"装填未开局"悬挂) |
| 暂停       | 不装填不判定;在途试用局跑完照判;分析照跑仅计数                                                         |

- 移除装填/采纳/丢弃按钮与"待决候选"徽标;兜底「重置策略」按钮(审计 actor:user)。
- 不变量零改动:局内快照、K/ε/冷却、审计;「无基线直转」收紧为仅 iteration=0。

---

## §4 反射 vs 纯反射

- **reflex**:Laya 每帧决策 + 规划器意图提示(subgoal)——域蒸馏;共享四条物理护栏(跳跃弧补全/停滞自愈/本能否决/采样)。
- **pure**:同护栏、零提示,Laya 凭观测件裸判断。**用途 = 消融基线**:reflex−pure = 提示价值;观测/消息设计演化后 pure 的提升 = "更好的数据能否替代提示"。日常默认 reflex,统计分模式,「单步/按需」时作升级阶梯 L2 对照(§6.7),不常开。

---

## §5 FloatingPanel 统一可拖动窗体

- `src/lib/ui/FloatingPanel.svelte`(全应用复用):props `title/testid/width/initial/persistKey/onclose`;标题栏拖拽(Pointer Events + setPointerCapture + touch-action:none)、折叠、点击置顶(模块级 zTop 计数)、位置持久化。
- 补充规格(analysis-15-5):Esc 只关最顶层 active 窗、焦点回 opener、`aria-expanded`;**<900px 退化为单窗**(新开替换旧,宽 min(96vw,width) 底部居中);persistKey `qx.mario.panel.<id>` 存 `{v:1,x,y,folded}`;键盘 ←→↑↓ 移动 ±16px;实施时跑对比度 ≥4.5:1 校验。
- MarioLab:`panel` 单值 → `openPanels` 集合,laya/settings/pm/evo 全部换装,多窗叠开。

### 5.6 日志分级(决策流必须先出日志)

- **先决**:删除 driver.onDecision 每拍日志(250ms 一条,10 秒刷满 buffer);最新一拍只活在 HUD 门控徽标 + laya 面板;异常拍(ESCALATE/stale)5s 限频进 evolve 档。
- 三档:`game`(slate)/`evolve`(violet)/`error`(red,恒显)+ 过滤 chip「全部/玩法/进化/异常」;行模板 `HH:MM:SS {icon} {text≤30}`。
- reason→短语映射放 evolution.ts 纯函数(`already-analyzed`→`已析过`、`death-repeat(sig)`→`重复死亡:sig`…),可测;JSONL 保留原始枚举。
- ⚠ 实施前先 grep e2e 对 log-stream 的文本断言,同步修改。

---

## §6(重心)每死全量 Laya 数据分析 v3

### 6.1 触发、幂等与时序(修 v1 现存 bug)

- 分析单元:deathKey = `${runId}#${attempt}`(一局 3 死 = 3 次独立分析);win/plateau 仍 run 级。
- **幂等标记移到成功之后**(修 R6:现状 UI 触发即标 analyzed,失败不回滚 → "每死必析"退化"每死一次机会"):`analyzed = outcome.ok` 才占键;run 级双查旧 analyzedRunIds(只读迁移,修 R9)。
- repeat 判定从"相邻局单值"改为**按签名计数**(sigCounts,滚动 20 局窗口;修 A-B-A 交替漏标)。
- death-repeat 载荷必带上次同签名**诊断+补丁**;上次分析失败则注入"上次分析失败(原因)",不给 Qwen 不存在的前提(修 R7 语义)。

### 6.2 队列与调度(AnalysisScheduler 纯逻辑,UI/e2e 复用)

```text
enqueue:同 deathKey 合并(merged++);queue cap6 → 溢出写 state.pendingAnalyses 落盘 backlog,不丢
优先级:first-death > 新签名 > repeat > win > plateau
runLoop(局间串行槽;局内帧不中断):
  先清 backlog 再收新任务;audit(analyze,start)
  refineViaQwen({signal: AbortSignal.timeout(60s)}),内部重试 1 次(attempts≤2)
  ok: 占幂等键 + counters + audit(analyze,ok,latencyMs)
  fail: attempts≤2 重入队;超限 audit(analyze,fail,errClass);连续 3 失败 → 熔断暂停(探活成功自动复位)
入队即深拷贝不可变快照(deathKey+report+policy+消息设计+champion updatedAt)——修"引用活对象/归因错位"
```

**分析硬门(产品决策,修订"异步不阻塞"旧口径)**:一局结束(3 死或通关)后,
**分析没出结果,下一局不得开始**——UI 的 开始/重开 按钮 await 队列排空
(按钮转圈禁用 + 日志"等待 Qwen 分析完成");Qwen 不可达时任务重试上限后
放弃,门有界自动放行(不卡死)。e2e 天然串行,天然满足。局内(第 1/2 死)
帧循环不受影响,分析仍只在局末触发。

- 补丁落盘**快照-rebase**:完成时 champion 若已变(用户/另一进程),对当前值重算 sanitize+limit+cooldown,diff 为空 → 审计 `propose-stale`;写 policy.json 前再比对 updatedAt(修 R11/R12;跨进程并发以"UI 与 e2e 不得同时跑"红线 + updatedAt 兜底)。
- **提案 cadence = run 级**:同 run 多次死亡分析合并为**一个**提案(originSigs 多值)——候选单槽 `pending` 下 3 死 3 提案互相覆盖是 v3 致命缺口(analysis-15-2 G2);K 局评估在途时新提案只审计不入队。
- 状态一致性:e2e saveState 改 tmp+rename 原子写(与 UI 同纪律);state 损坏时从 audit/history 重建 championScore,损坏文件存 .corrupt + 审计 reset 行;写序**先审计 append 后 state 写**,启动重放对账(修 R7/R8)。

### 6.3 载荷:完整过程 + Laya 收发明细(修正版预算)

数据链路(实读核实,analysis-15-3):

- **stateJson 已存在**(brain.ts Decision.stateJson,未进 JSONL)——只需补 `questionsJson`(buildQuestions 提出循环外,Decision 加一字段);RE_SENSE 重发同一份,捕获一次即准。
- driver 环形缓存 24 条 `SentRow={row,state,questions,tick}`,**decide 返回后、stale 分支之前插入**(stale 拍消息已发出),tick 发射时捕获,**按 attempt 分段**(修 R15 串味);inputFor 新增 heldTicks 计数器;死亡上下文 slice(-8)→12 并从 sent 环回填 state 文本。
- **obsStateExtra 需 brain.decide 加第 5 参 `extra`**(brain 无历史,lastAction/heldTicks/stallTicks 全在 driver)。

每死载荷(按实测 token 计数裁剪,不按表上限;24k chars ≈ 6k token):

| 块                             | 内容                                                                                                                                                                 | 实估   |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| ① 全局 digest                  | processDigest                                                                                                                                                        | ~700   |
| ② 本 attempt 轨迹 CSV          | tick,col,action,conf,gate,applied,note;>120 行确定性采样(保首 8 行+全部异常行,step=ceil(n/slots))                                                                    | ~1.6k  |
| ③ 死前 12 拍 Laya 明细         | 每拍两行:`t1187 col45.2 \| 37% cn5 \| prof… \| thr… \| +2.5/0 g1 R \| sub…` + `=> run_right .71 EX 210ms`;**默认不带 questions 行**(仅 obsHintStyle 近 3 局变更局附) | ~1.2k  |
| ④ 全程异常行(cap 6,超出降 CSV) | 低置信<0.15/门控翻转/空转/stale,带 state 明细                                                                                                                        | ~0.7k  |
| ⑤ 其它 attempt 摘要            | 每 4 拍一行 + 各死亡点上下文                                                                                                                                         | ~0.5k  |
| ⑥ 上下文                       | 手册/历史5/当前策略+消息设计(入队时冻结)/上次同签名诊断+补丁/近 3 条 insight                                                                                         | ~0.75k |

合计 ≈5.4k ✓(v1 草案表上限相加 6.6k 自相矛盾,已修正)。
**裁剪顺序(修 v1 §8 含糊)**:PROTECTED ③④⑥ 只降密度不归零(③ 最低 8 拍);TRIMMABLE ⑤→②→① 依次缩;确定性公式 + 每块 `(sampled n/N)` 元数据;全部裁完仍超 → 任务 overBudget-failed,不送脏载荷。死因证据永不被裁。
精简(代码文案直改,不涉红线):profile RLE(`13*10`,省 45%)、pose 压缩(`+2.5/0 g1 R`)。

### 6.4 Laya 消息内容设计沙箱(变体注册表)

**实现前置缺口(实读证实,必改)**:`limitPatch`(evolution.ts L308-323)变更收集只认 number/bool/Window——5 个枚举字段(string)会被**静默丢弃且无 issue**,消息设计沙箱即死代码。必须补枚举分支 + 维度分组(§6.7 单变更原则)。

policy 新增 5 字段(确切文案全表见 analysis-15-3;OBS_ENUMS/OBS_EXTRA 常量表校验,表外值回默认+issue):

| 字段                  | 变体                                  | 默认    | 备注                                                                                             |
| --------------------- | ------------------------------------- | ------- | ------------------------------------------------------------------------------------------------ |
| obsThreatFormat       | named / rows / off                    | named   | rows=`goomba d2 r13` 补**同行纵向关系**(veto 判据同排,named 无行号——评审采纳的遗漏维度,零新字段) |
| obsProgressStyle      | full / pct / off                      | full    |                                                                                                  |
| obsHintStyle          | full / terse / off                    | full    | off=键留值空串,laya-server 合约需实测,兜底 `'-'`                                                 |
| obsInstructionVariant | default / concise / checklist         | default | `{n}` 插 obsProfileCols 防 concise 说谎;**checklist=文字版数值补丁,默认不开**,按设计变更评估     |
| obsStateExtra         | ⊆[lastAction,heldTicks,stallTicks] ≤2 | []      | 拼 `state.ctx` 行                                                                                |

防输入漂移(Laya conf 分布与 state 文本强耦合,门控 0.16 按 v2-obs 校准):

- 审计行加 `patchKind: numeric|design|mixed`;**limitPatch 拒绝 mixed**(设计+数值同窗不可归因);
- 设计变更评估窗 `DESIGN_EVAL_RUNS=5`(数值仍 3),ε 不变;设计字段回滚自动进冷却(按字段名,零改动);
- 漂移局提示:设计变更后第 1 局 avgConf 漂移 >0.05 或 execRate 变化 >25% → 提示"漂移局,第 2 局起计分"(先提示不丢弃)。
- 系统提示给选型指引(每字段 ≤2 句 + "单次 patch 至多 1 个消息设计字段"),选型全文见 analysis-15-3。

### 6.5 insight 必填 + 可证伪协议

- 结构:`{kind:'claim'|'maintain'|'refuted', claim≤150字, field?, metric∈[winRate,repeatRate,execRate,staleRate,avgConf,p50ms], direction, evidenceIter}`——缺 metric/证据视同空,走重试;重试仍不合格落 `maintain`(单独计数,不占经验额度)。
- 去重:与近 5 条词元 Jaccard ≥0.6 → 不追加,原条 confirmCount+1(重复确认也是信号)。
- 出清(治"只进不出"):§6.7 账本结算后回扫,方向被 sigDelta 反驳 → 标 `refuted`;淘汰序 refuted→maintain→FIFO。
- 命中率(方向与 sigDelta 一致占比)进统计;<30% 触发提示词复盘。(顺带:v1 的 insightCount 参数收了没用,一并落实。)

### 6.6 Qwen 输出 schema v3 + 快慢脑边界硬化

```jsonc
{ "playbook": "…",                        // ≤1000 字,不变
  "policy_patch": { … },                  // 数值 + 消息设计字段(单维度,§6.7)
  "death_diagnosis": [ {"deathKey":"r…#2","rootCause":"…","responsibleTick":1231,"fix":"…"} ],
  "insight": { … } }                      // §6.5 结构
```

硬化 4 条(analysis-15-4):①未知顶层键 → 丢弃 + 审计 issue(越权可观测,不静默);②policy_patch 字段过静态性声明表(类型/值域/钳制),含 tick/条件语义直接拒;③death_diagnosis 单向流——只进载荷⑥+审计,driver/brain 禁 import;④提示词软声明("你是局间策略优化者,不是逐拍玩家"),载荷③ 加"归因素材,不得产出逐拍指令"前缀。diagnosis.deathKey ∈ 载荷清单校验,幻觉丢弃 + badJson 计数。

### 6.7 归因、升级阶梯与回滚写回(自进化的"不断进化"保障)

- **变更-效果账本** versions/ChangeLedger:`{changeId, vid, fields, dimension∈[gate,window,obs-geometry,message-design], originSigs, deathKeys, verdict, scoreDelta, sigDelta}`;**单变更原则代码强制**(数值与消息设计不得同车;跨维度按幅度留一,余丢+issue)。
- 归因规则(确定性,非 LLM):commit 后滚动 W=5 局按签名结算 repeatBefore/After,仅改善才记"治了";responsibleTick 交叉验证。
- **签名级升级阶梯**(治死循环):同签名 ≥3 次提案未改善 → L1 强制换维度(数值→消息变体,如 gap 死→rows)→ L2 pure 对照局(pure 也死=观测数据问题锁定 obs*;不死=提示层问题锁定 hint/instruction)→ L3 冻结签名让预算(版本 restore 可解冻)。
- **回滚写回**(analysis-15-2 G10):回滚时向 playbook 追加"iter-N 假设 X 已回滚(Δ=…),勿重复提案";提案上下文注入近 3 条被否决记录——"教训不回滚"改为"教训可证伪"。

---

## §7 长期治理(自进化的"健全"保障)

| 对象            | 现状                | 决定                                                                          |
| --------------- | ------------------- | ----------------------------------------------------------------------------- |
| insights.md     | 2400 字             | **升 4800**;重复确认 ≥3 次自动升人保段                                        |
| playbook.md     | 1600 字             | 不动(战术本就该滚动)                                                          |
| evolution.jsonl | 无限追加            | >10MB 轮转 .1(轮转≠删除);parseAuditLines 版本白名单 [1,2](防升版静默丢行)     |
| history.json    | 50 局               | 升 200(Δ 趋势与北极星窗口需要)                                                |
| state.json      | —                   | 加 schemaVersion;只加不删,旧字段保留只读一版                                  |
| runId           | Date.now+进程内 seq | 排序/去重改用 iteration/runSeq 序键;runId 加 boot 随机前缀(时钟回拨/重启防撞) |

- **北极星**:滚动 10 局通关率(<0.5×此前最优窗口报警);次级:签名收敛速度(单签名 >15 局未收敛触发升级阶梯)、insight 命中率、漏斗健康(commit 后 5 局内回退 >30% → ε 复盘)。
- **退化自动处置**:连续 2 批中位分下行 → 自动 restore 最优版本 + 冻结提案 COOLDOWN_ITERS 局 + 审计 `regression-restore`。
- **版本账本** versions.jsonl(cap 20):`{vid, iter, policy 全量, patch, originSigs, verdict, score}`;UI 版本表按分排序一键回退;restore 后 championScore=null 强制重测、提案须带 base=championVid 防分叉。

---

## §8 领域无关内核(终极目的落地形态)

沉淀的是 **Laya 跨领域使用经验**,马里奥只是第一试验场:

- **进 core(lib/evolution-core)**:diffPatch/limitPatch/冷却/judgeCandidate/median;触发器协议(deathKey→unitKey);EvoState 簿记;审计;appendInsight+人保段;**变体注册表框架**(键→枚举+默认+sanitize,内容由领域注册);AnalysisScheduler;policy 信封版本化。
- **留 adapter(马里奥专用)**:engine/world/planner/renderer;物理护栏;encodeState;postmortem 指标语义;死亡签名。
- `DomainAdapter<P,R>` 接口:policySchema(含静态性声明)/scoreRun/unitsOf/digestOf/variants。下一领域 = 新 adapter + 复用 core 全部纪律。
- 时机:**不阻塞 v3**,但 v3 新代码(队列/触发清单/counters/machine)必须 core 兼容写——纯函数、零 mario import。insights 加 `domain:` 标签,跨域结论可检索。

---

## §9 实施计划

**顺序(依赖序;更新点加粗)**:

1. policy.ts:5 消息设计字段 + OBS_ENUMS/OBS_EXTRA + **静态性声明表**
2. brain.ts:变体拼装、`decide` 第 5 参 extra、**questionsJson 返回**、RLE/pose 精简
3. driver.ts:SentRow 环(24 条/attempt 分段/发射时 tick)、heldTicks、死亡上下文 12 拍
4. postmortem.ts:exportDeathTrace + 异常行提取 + **确定性裁剪(D5)**
5. evolution.ts:v3 触发清单/deathKey 幂等(**成功后占键**)/counters/**limitPatch 枚举+维度分组**/sigCounts/ChangeLedger/reason 短语映射
6. qwen.ts:buildDeathPayload(按实测 token 计数)/schema v3 解析(**未知顶层键审计**)/insight 可证伪校验/超时 AbortSignal+熔断
7. **machine.ts + AnalysisScheduler(新,纯逻辑,UI/e2e 复用)**;e2e saveState 原子写 + catch 补 recordRun
8. MarioLab:三态开关/队列面板/统计块/FloatingPanel/HUD DOM/日志三档/**删除决策拍日志**
9. 测试:枚举 sanitize、变体拼装快照、幂等时序(失败不占键)、队列合并/熔断、裁剪确定性、insight 可证伪、混车拒绝、**审计重放对账**

**提交拆分**:①HUD+日志分级 ②统计 ③状态机+全自动 ④FloatingPanel ⑤v3 数据链路(policy/brain/driver/postmortem) ⑥v3 进化与归因(evolution/qwen) ⑦接线(UI/e2e/machine) ⑧docs 校订。

**实施前检查**:grep e2e 对 log-stream 文本断言;实测 laya-server 空 criteria 行为(obsHintStyle=off 兜底 '-');实测 Qwen ctx(裁剪顺序不依赖实测,但 ③ 12/8 拍由实测定)。

---

## §10 已拍板与开放问题

**已拍板**:跨进程并发=红线约定+updatedAt 兜底(不上文件锁);无基线直转收紧 iteration=0;backlog 重试=局间槽先清;core 抽取放 v3 验证后(v3 兼容写);insights 4800/history 200;checklist 默认关;文案两层不互渗;**分析硬门**(用户决策,推翻"异步不阻塞"旧口径):失败/通关的分析没出结果,下一局不得开始——UI 开始/重开 await 队列排空,Qwen 不可达时重试上限后有界放行;e2e 天然串行。

**开放(实施中按需回签)**:

1. K=3 中位统计功效:分数方差超阈值是否自动加跑?
2. 单变更原则拉慢探索:维度内多字段(如 gate 三兄弟)是否允许?待 A/B 数据。
3. sigDelta 窗口 W=5 小样本:是否随签名死亡频率自适应?
4. 结构化 insight 对本地 Qwen JSON 稳定性要求高:先 10 局抽样测合规率再定松紧(temperature 0.2→0.3?)。
5. pure 对照局是否入主 history(建议独立 abRuns 口径,不污染通关率)。
6. **升级出口**:诊断链多轮无效时缺"人工/云端代理改代码"通道——v3 先做 L3 冻结,通道二期设计。
7. UI 暂停/document.hidden 时队列是否继续消化(倾向继续,需真机验证后台网络节流)。

---

### 附录:评审源文档

[15-1 可靠性](analysis/15-1-可靠性分析.md) · [15-2 进化质量](analysis/15-2-进化质量分析.md) · [15-3 Laya消息设计](analysis/15-3-Laya消息设计评审.md) · [15-4 架构健全性](analysis/15-4-架构健全性评审.md) · [15-5 UI文案与可观测性](analysis/15-5-UI文案与可观测性.md)
