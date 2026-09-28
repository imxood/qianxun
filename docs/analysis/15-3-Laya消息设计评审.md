# docs/analysis/15-3 · Laya 观测数据与消息内容设计 — 落地性评审(3号)

## 结论摘要

1. **stateSent 已半成品**:`Decision.stateJson` 已存在(brain.ts:139/226)且未进 JSONL——docs/15 §6.3 的"stateSent"实为**复用现名 + 补 questionsJson**,一行返回值 + 一个类型字段。
2. **消息变体五字段全部可落地**,但 `obsHintStyle=off` 依赖 laya-server 对空串 criteria 的容忍(候选集靠键枚举),需一次合约验证,兜底值 `'-'`。
3. **载荷预算表自相矛盾**:docs/15 §6.3 各块预算相加 = 6.6k > 头条 6k;按实测 token 估(§下文)实际约 5.2k,能进,但前提是 **③ 死前明细默认不带 questions 行**。
4. **每拍 state 实测 ≈110-125 tok**,在 300 目标内;profile 数字串 RLE 压缩可再省 ~45%,pose 压缩再省 8 tok——两者是代码文案直改,不涉红线。
5. **防退化风险成立且必须处理**:conf 分布与 state 文本格式强耦合(门控 0.16 按 v2-obs 校准),变体切换 = 输入漂移。需补丁分类(numeric/design 禁混窗)+ 设计候选评估窗 3→5 局 + 复用既有字段冷却。
6. **被遗漏维度定为"同行纵向关系"**:veto 判据是同排(driver.ts:326),而 named 威胁只有 px 距离无行号——`rows` 变体的 `r13` + pose 自身行号即闭环,零新字段。威胁时距(ticks)次优先,并入 rows 的 goomba 尾巴。

---

## stateSent 与缓存改动点(文件:行级)

**可行性**:`decide()` 中 RE_SENSE 循环(brain.ts:184-191)重发**同一份** state/questions,故捕获一次即准确;state/questions 均为纯内存对象,不入 JSONL,符合 §6.7 红线。

| # | 位置 | 改动 |
|---|---|---|
| 1 | brain.ts:181 | `const questions = buildQuestions(candidates)` 提到 while 循环外;`ask(state, candidates)` 改 `ask(state, questions)` |
| 2 | brain.ts:231-238 | `ask` 签名加 `questions` 参数,`body: JSON.stringify({ state, questions })` 用之 |
| 3 | brain.ts:129-141 / 217-228 | `Decision` 加 `questionsJson: string`;返回处加 `questionsJson: JSON.stringify(questions)`。**stateJson 不新增**(已存在) |
| 4 | brain.ts:83-104 | `encodeState(s, world, intent, mode, extra?: ObsExtra)`;`ObsExtra = {lastAction?, heldTicks?, stallTicks?}`;按 `obsStateExtra` 拼 `state.ctx`(见注册表)。extra 必须由 driver 传——brain 无历史,driver 持有全部三者 |
| 5 | driver.ts:141-148 | record 加 `sent: SentRow[]`;`SentRow = {row: PmDecisionRow; state: string; questions: string; tick: number}` |
| 6 | driver.ts:366-390 | `decide()` 返回后**立即**(stale 分支之前)push sent:`tick` 取发射时的 `s.tick`(attempt 守卫过后 s 已不可信,不能事后补);`length > 24 → shift()`。**stale 拍也要入环**——消息已发给 Laya,④ 异常块需要它 |
| 7 | driver.ts:194-222 | `reset()` 清 `sent: []` |
| 8 | driver.ts:254-323 | inputFor 增加 `heldTicks` 计数(held 换量子归零,否则 ++),供 ObsExtra |
| 9 | driver.ts:446-453 | 死亡上下文 `slice(-8) → 12`(§6.6),并从 sent 环按 tick 回填 state 文本 |

## 变体注册表规格(表格 + 确切文案)

### 校验与值域(policy.ts)

- 常量表导出(BOUNDS 之外新增两张,枚举不走数值钳制):
  `OBS_ENUMS = {obsThreatFormat:['named','rows','off'], obsProgressStyle:['full','pct','off'], obsHintStyle:['full','terse','off'], obsInstructionVariant:['default','concise','checklist']}`;`OBS_EXTRA = ['lastAction','heldTicks','stallTicks']`。
- sanitize 规则:非字符串/表外值 → 回默认 + issue(`obsThreatFormat: "xyz" → named(回默认)`);`obsStateExtra`:非数组丢弃,逐项过滤白名单、去重、截前 2 项,issue 列出被丢项。
- `limitPatch`/`MAX_PATCH_FIELDS=3`/冷却按 key diff 工作,**零改动**即覆盖新字段;`MAX_NUMERIC_STEP` 对枚举天然跳过。

### 逐字段确切文案

**obsThreatFormat**(默认 named;≤3 条,无则 `none`)
| 值 | 示例行 |
|---|---|
| named(现 threatText 原样) | `goomba dx=32px \| gap dx=4t w=3 \| pipe dx=96px h=2` |
| rows(瓦距+行号,补纵向) | `goomba d2 r13 \| gap d4 w3 \| pipe d6 h2` |
| off | 字段省略 |

**obsProgressStyle**(默认 full)
| 值 | 示例行 |
|---|---|
| full(现) | `x=1234/3376 37% coins=5 time=287 attempt=3` |
| pct | `37% coins=5 time=287 att=3` |
| off | 字段省略 |

**obsHintStyle**(默认 full=现 ACTION_HINTS,planner.ts:40-51)
| 值 | 规格 |
|---|---|
| terse(≤3 词,物理约束保留括注) | `idle:wait / right:walk right / run_right:run right (default) / jump:jump up / jump_right:short jump right / jump_run_right:run-jump right (wide gaps, tall pipes)` … |
| off | criteria 键保留、值空串(选项集靠键枚举)。⚠ 依赖 laya-server 容忍空描述;若降智则兜底 `'-'` |

**obsInstructionVariant**(全文;`{n}` 插 obsProfileCols,避免 concise 在 cols=8 时说谎)
| 值 | 全文 |
|---|---|
| default(现,brain.ts:113-116) | `Mario reflex step. profile{n} = first solid tile row of the {n} tiles ahead (13 = ground level, 15 = open pit, 9 = floating block row). Threats list named hazards with distance. Choose exactly one next action.` |
| concise(~20 tok) | `Pick one action. profile{n} = first solid row ahead (13 ground, 15 pit, 9 block).` |
| checklist(~45 tok) | `Choose one action.\n1. hazard <=2 tiles & same row -> jump over\n2. gap ahead -> jump_run_right\n3. clear -> run_right\nprofile{n}: 13 ground, 15 pit, 9 block.` |

⚠ checklist 把战术写死进代码文案——红线不破(非 Qwen 写),但**行为效果等同数值补丁**,评估必须按设计变更对待(见防退化)。

**obsStateExtra**(默认 `[]`;拼一行 `state.ctx`,仅含选中项)
| 项 | 行文 | 来源 |
|---|---|---|
| lastAction | `last=run_right`(guard 拍记 `last=guard`) | driver.lastDecision |
| heldTicks | `held=12` | driver 新计数器 |
| stallTicks | `stall=180` | driver.stallTicks(已有,driver.ts:137) |

## 载荷格式与裁剪(示例 + 伪代码)

### ③ 死前 12 拍明细(每拍两行,~85-110 tok)

```
t1187 col45.2 | 37% cn5 tm287 a3 | prof 12 13 13 15 15 15 13 13 13 13 13 13 13 13 13 13 | thr gap d4 w3 pipe d6 h2 | +2.5/0 g1 R | sub jump_gap[jump_right,jump_run_right]
=> run_right .71 EX 210ms
```

| 字段 | 来源 | 格式 |
|---|---|---|
| t | 发射时 s.tick(新增) | `t1187` |
| col | mario.x/TILE | 1 位小数 |
| progress | state.progress 按变体原样 | `37% cn5 tm287 a3` |
| prof | profile16 join | 数字串 |
| thr | threats 按变体 | 注册表文案 |
| pose | 压缩位姿 | `+vx/vy g{0,1} {L,R}` |
| sub | intent | `type[cand1,cand2]`;pure 模式缺失时决策行附 `cand …` |
| => 决策 | Decision | `action .conf GATE ms`;gate 缩写 EX/RS/ES,stale 后缀 `!`,guard 拍记 `guard` |

q 行(候选提示原文)默认**不落**;仅 obsHintStyle 近 3 局内变更过才附(诊断"提示误导"用)。

### ② 全量轨迹 CSV

```csv
tick,col,action,conf,gate,applied,note
1187,45.2,run_right,0.71,EXECUTE,1,
1212,46.1,jump_right,0.09,RE_SENSE,1,
1225,46.1,idle,0.04,ESCALATE,0,stale
```

### 裁剪算法(确定性)

```
function isAnomaly(r): r.applied === false || r.gate != 'EXECUTE'
                     || (r.conf ?? 1) < 0.15 || !!r.note
function trimTrace(rows, cap = 120):
  if rows.length <= cap: return rows
  keep  = rows.filter(isAnomaly)            // 全保,可击穿 cap(异常 > 预算)
  head  = rows.slice(0, 8).filter(!keep)    // 首 8 拍
  rest  = 其余行,按 tick 升序
  slots = max(12, cap - keep.size - head.length)
  step  = ceil(rest.length / slots)
  picked = rest.filter((_, i) => i % step === 0)
  return (head ++ picked ++ keep) 按 tick 稳定排序
```

④ 异常行带 state 明细:0.8k 预算只够 ~6 条全量行;规格 = **cap 6,超出部分降级为 CSV 行**(thr+progress 两件,~45 tok,再取 8 条)。

## token 预算估算

现 profile16 + named 全开,单拍 state 实测估(字符/4 折算):

| 件 | 文案 | 估 tok |
|---|---|---|
| encoding 键 | `"encoding":"v2-obs"` | ~5 |
| progress full | `x=1234/3376 37% coins=5 time=287 attempt=3` | ~16 |
| profile 16 数字 | `12 13 13 15 15 15 13 13 13 13 13 13 13 13 13 13` | ~20 |
| threats named(3 条) | `goomba dx=32px \| gap dx=4t w=3 \| pipe dx=96px h=2` | ~28(`none`≈2) |
| mario | `vx/vy=+2.5/+0.0 on_ground=1 face=R` | ~16 |
| subgoal | `jump_gap gap w=3 -> candidates [jump_right, jump_run_right]` | ~18(空 ~12) |
| JSON 结构 | 键名+引号+逗号 | ~10 |
| **合计** | | **≈113-125**,questions 另 ~90-110 |

6k 预算核对:① digest ~700 + ② 120 行 CSV ~1.6k + ③ 12×~100 ≈1.2k + ④ 6 条 ~0.7k + ⑤ ~0.5k + ⑥ ~0.75k ≈ **5.4k ✓**;但 docs/15 表面上限相加 6.6k,若 ③ 附 q 行(+35×12)即破 6k——**载荷组装须按实测 token 计数裁剪,不按表上限**。

精简建议(代码文案直改,不涉红线):
1. profile RLE:`12 13*2 15*3 13*10`,~20→10 tok(省 45%,对 ③ 十二拍累计省 ~120)。
2. pose 压缩:`+2.5/0 g1 R`(16→8 tok),on_ground→g、face→L/R。
3. instructions 选 concise 变体省 ~35/拍;`encoding` 键可只在审计行保留、不进 state(省 5,低优先)。

## 选型指引(qwen.ts system,L51 后追加)

```
消息设计字段(值只能取注册变体,不得自创文本):
- obsThreatFormat named|rows|off:named=名+px 距离,rows=瓦距+行号。空转或"看不见威胁"型死亡多 → rows
  或调大 obsThreatsLookPx;威胁识别正常 → 别动。
- obsProgressStyle full|pct|off:pct 去掉绝对像素,一般够用;通关在望 → 不许动。
- obsHintStyle full|terse|off:token 紧张且动作选择清晰 → terse;跳管/宽坑失败率回升 → 回 full。
- obsInstructionVariant default|concise|checklist:concise 省 token;checklist 内置"威胁就跳"规则,
  仅开局即死模式考虑;不确定 → default。
- obsStateExtra ⊆[lastAction,heldTicks,stallTicks] ≤2:门控抖动/空转多 → stallTicks;动作反复横跳
  → lastAction+heldTicks;无对应异常 → 留空。
纪律:单次 patch 至多 1 个消息设计字段(与数值字段合计仍 ≤3);设计字段改后至少观测完整评估窗才许下结论;回滚过的字段按既有冷却处理。
```

## 防退化与开放问题

**变体切换会导致 Laya 当局表现突变吗?会。** Laya 是窄域蒸馏小模型,conf 分布与 state 文本强耦合,GATE_EXECUTE=0.16 是按 v2-obs 校准的(docs/11 §6"OOD 域必须实测重校");换 threats/progress/instructions 文案 = 输入分布漂移 → avgConf/execRate/thrash 全部失真,并与数值补丁效果混叠,判错归因。

**最小机制(不建议更重的)**:
1. audit 行加 `patchKind: numeric|design|mixed`;`limitPatch` 拒绝 mixed——设计+数值同窗无法归因。
2. 设计变更候选评估窗 `DESIGN_EVAL_RUNS = 5`(数值仍 3);ε 不变。
3. 设计字段回滚**自动进冷却**:cooldowns 按字段名工作,evolution.ts:373-381 零改动即覆盖 `obs*` 键。
4. 漂移提示:设计变更后第 1 局,avgConf 漂移 >0.05 或 execRate 相对变化 >25% 时,复盘规则层提示"输入漂移局,指标不计入候选评估"(第 2 局起计分)。先做提示,不做自动丢弃。

**遗漏观测维度评估**:
- **同行纵向关系(采纳)**:veto 判据是同排 z 差 ≤8px(driver.ts:326-334),但 named 只有 px 距离、无高度——"直冲同排小怪"恰是 veto 存在的理由。`rows` 的 `r13` + pose 加自身行 `r12` 即闭环,零新字段、~2 tok。已并入上文注册表。
- **威胁时距 t(次优先)**:postmortem.ts:311-313 规则层已两次建议 `hazardDist(ticks)`;但 t=dx/vx 静止时无意义,仅 goomba 有效 → 不单列字段,作 rows 变体 goomba 行可选尾巴 `goomba d2 r13 t3`(按 vx 折算拍数)。先观测 rows 上线后的 veto 率走势再定。
- 不采纳:相机 x(与 mario.x 共线)、帧序列(超文本预算)。

**开放问题**:
1. `obsHintStyle=off` 空 criteria 的 laya-server 行为需一次实测;兜底 `'-'`。
2. checklist 的战术内置是否与"Qwen 不写战术"精神冲突:文案在代码(红线不破),效果按设计变更评估即可,但**默认不开**。
3. ③ 是否附 q 行的预算联动(仅在 obsHintStyle 变更局附)需在 buildDeathPayload 落实为规则而非提示词。
4. `obsThreatsLookPx` 与 `rows` 变体同时变更时的归因污染——选型指引已约束"单 patch 至多 1 个消息字段",测试需覆盖该拒绝路径。
