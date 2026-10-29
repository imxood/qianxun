# 10 · Laya 街机实验室:完全用 Laya 玩游戏的设计与实测

> 呼应 TypeSafe AI 的 JEV「Tetris Reflex Lab」(System 1 反射 × System 2 规划),
> 用**我们自己的 Laya** 复刻同一架构。已落地为千寻「游戏」页的第一个游戏:
> **Laya Reflex · 俄罗斯方块**。
>
> ⚠️ **2026-10-04 事实订正**(详见 [docs/16 §1](./16-马里奥v2与Laya推理提速.md)):
>
> 1. **JEV 从未开源**——它是 TypeSafe AI 的闭源托管 API,没有权重、没有 HF 仓库。
>    **开源的是 Laya**(Convai Innovations,Apache-2.0)。所以"替代 JEV"实际等于
>    "选一个开源平替",而 Laya 就是那一个,本项目的选型没有错。
> 2. "Tetris Reflex Lab" 这个 demo 名称**查不到**。TypeSafe 官方 demo 是
>    Doom / Wikiracing / 工单三例;社区有 "Jev Loses at Tetris 62.8%" 的视频。
>    §1 的"截图逆向"表述本身是对的,但 demo 名称对不上。
> 3. "Laya 碾压 Jev 近 20 倍"是**本地推理 vs 跨境云端往返**(M5 Pro 15.3ms vs
>    298.1ms),同硬件口径是 **6–8 倍**。

## 1. JEV 是怎么实现的(截图逆向)

JEV 是 TypeSafe AI 的 System-1 非自回归决策模型(与 Laya 同类,API 形状一致:
`{state, questions:{choice/score/boolean}}`)。其 Tetris Lab 三栏循环:

```
01 SENSE → STATE           02 DECIDE → JEV REFLEX          03 ACT → GAME LOOP
────────────────           ─────────────────────           ──────────────────
当前块/位置/落点            conf=96.8% → EXECUTE            棋盘渲染 + 分数
System 2 规划意图           动作概率分布(7 动作)            决策频率 150ms(~6.7Hz)
(target rot·x, plan         置信度门控三分支:               重力/延迟模拟(70-500ms)
 score -250.05 · 0.2ms)     >0.85 EXECUTE                   [人玩 / Jev 观察位]
板面压力(洞/崎岖/危险)      0.5~0.85 RE-SENSE 再感知
                            <0.5 ESCALATE → System 2 重规划
```

关键实现要点:

1. **System 2 是经典求解器**(非学习):Dellacherie 特征(洞/崎岖/井)枚举
   所有 rot×x 放置,单次 0.2ms——截图里的 plan score/eval 字段就是它;
2. **System 1 反射模型只做单步决策**:看状态选 7 个离散动作之一 +
   输出置信度(它对游戏域蒸馏过,所以 conf 高达 96.8%);
3. **置信度门控三分支**:高置信直接执行;中置信重新感知;低置信升级
   System 2 重规划——三层都在 UI 上实时可见。

## 2. Laya 可行性对照

| 要素          | JEV Lab            | Laya 方案                    | 结论                   |
| ------------- | ------------------ | ---------------------------- | ---------------------- |
| System 1 原语 | choice+confidence  | choice/score/noul+confidence | ✅ 同构                |
| 调用延迟      | 0.1ms(其模型极小)  | 26ms(单题)/280ms(棋盘长文本) | ✅ 决策频率 1-6Hz 可玩 |
| System 2      | Dellacherie 求解器 | 同款(JS 枚举+el-Tetris 权重) | ✅ 已实现              |
| 游戏域置信度  | 96.8%(蒸馏过)      | **0.169(未蒸馏,OOD)**        | ⚠ 见 §4                |
| 服务形态      | 本地引擎           | laya-server sidecar(已有)    | ✅ 零新增基建          |

> 📌 **延迟一行已被 2026-10-05 的实测取代**(见 [docs/16 §4.4](./16-马里奥v2与Laya推理提速.md)):
> 本机 i9-14900HX / fp32 / 8 线程实测,延迟 ≈ **17 ms 固定 + 0.5 ms/token**。
> 棋盘长文本(≈220 token)在开启 ORT 三个开关后是 **141 ms**,不是 280 ms。
> 马里奥关的完整 state 同样是 217 token / 141 ms。**int8 量化尚未兑现**。

## 3. 已落地架构(千寻「游戏」页)

```
侧边菜单「游戏」→ GamesPage(注册表式列表,可持续加游戏)
  └─ Laya Reflex · 俄罗斯方块(TetrisLab.svelte)
       ├─ engine.ts  纯 TS Tetris 核心 + System 2 求解器(可单测)
       ├─ brain.ts   Laya 决策端:state 文本化(棋盘 20 行/当前块/规划目标/
       │             板面压力)+ questions(action choice + danger score)
       │             + 门控状态机(EXECUTE / RE_SENSE×2 / ESCALATE)
       └─ TetrisLab  三栏 UI(canvas 棋盘 + S2 目标虚线 + 动作概率条 +
                     决策日志流 + 统计 + 频率/重力/门控阈值滑杆)
```

驱动模式(实验开关):

- **reflex**(默认):System 2 给目标写入 state,Laya 逐步选动作逼近;
- **pure**:不给目标,纯 System 1 看盘——检验 Laya 原始水平;
- **solver**:求解器直控(对照基线,不经 Laya);
- **human**:键盘人玩。

## 4. 关键实测与校准(诚实数据)

- 游戏棋盘文本对 Laya 是 **OOD 域**:空板 T 块的 action conf≈**0.169**
  (工单域 0.9+);原 JEV 阈值(0.85/0.5)下会全部 ESCALATE。
- 处理:默认阈值按实测校准为 **0.16 / 0.12**,并暴露为 UI 滑杆实时可调;
  ESCALATE 时 System 2 保证游戏始终成立(置信度门控的语义因此真实可见:
  「Laya 不自信的一步交给规划器」)。
- 真机验证(CDP 探针 `e2e/probe-laya-tetris.ts`):进入游戏 → 开始 →
  EXEC 分支执行(conf 24.4%)、决策频率 ~1.6 QPS(单步 ~520ms,
  棋盘长文本推理)、概率分布/日志流/目标虚线全部工作。
- Laya 实际玩的水平:能推进游戏但会堆出洞(未蒸馏的诚实表现)。

## 5. Phase 2 路线:让 Laya 真正会玩(蒸馏)

> ⚠️ **2026-10-04 更新:蒸馏从"可选改进"升级为"前置条件"。**
> 两条独立证据都指向同一个结论:
>
> - Laya **base** 权重在官方 benchmark 上是 0.362,低于 0.461 的多数类基线。
>   model card 原话:_"Laya is a fast base to specialise, not a zero-shot decision engine."_
>   宣传的 0.766 来自**在该 benchmark 训练集上微调过的 checkpoint**。
> - Laya 自家仓库的中文飞书风格 benchmark:`laya-multilingual` **20/64 vs Jev 64/64**。
>
> 也就是说:当前 reflex 模式下 **System 1 从不真正"赢"**,100% 靠 System 2 兜底。
> 这是诚实的现状,不是 bug;但要拿到"System 1 真的会玩",只能蒸馏。

1. **数据**:求解器自对弈生成 10 万步 `(board_state, best_action)` 标注
   (JSONL,复用 laya-server /predict 的 questions 形状);
2. **微调**:receptron/laya 开源训练栈(`.tmp/laya-export` 已克隆)对
   mmBERT-base 头做 LoRA/全参微调,domain 从工单扩到棋局;
3. **验收**:游戏域 conf 提升到 0.7+、纯反射模式(pure)存活行数显著提升;
4. **上线**:微调 checkpoint 走现有 `models/laya-*` bundle 流程,零代码改动。

## 6. 扩展游戏的想法(注册表加一行即可)

- **2048**:每次移动方向 = choice(4 criteria);合并价值 = score;
- **贪吃蛇**:三向 choice(直行/左转/右转)+ 食物方位文本化;
- **打地鼠/反应类**:noul(是否出现)+ 反应延迟天然匹配 Laya 毫秒级;
- **双 Laya 对战**(五子棋):Laya(choice 落点) vs Laya(score 局面评分)。
