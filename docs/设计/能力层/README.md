# 能力层 (14 缺口)

> 状态: P0 (5) + 缺口 12 已落地 v0.3, P1 (8) 待启动 | 适用范围: 千寻 v2 之上叠加的 14 个能力缺口 | 最后更新: 2026-06-12

## 编号规则

按 **重要性 + 实施顺序** 编号, 数字越小越先做:

| 等级 | 编号范围 | 含义 |
|---|---|---|
| L2 P0 必做 | 01-05 | 没它核心功能跑不起来 |
| L3 P1 ROI 高 | 06-10 | 投入少, 收益显著 |
| L4 P1 ROI 中 | 11-12 | 投入适中, 收益明显 |
| L5 P1 投资大 | 13-14 | 复杂度高, 风险大 |

## 缺口清单

| 编号 | 名称 | 等级 | 借鉴源 | 状态 |
|---|---|---|---|---|
| 01 | [Hook 退出码 + 熔断](./01_Hook退出码与熔断.md) | L2 P0 | octos | ✅ 2026-06-10 (a6e9861) |
| 02 | [LLM 错误分类与恢复](./02_LLM错误分类与恢复.md) | L2 P0 | hermes-agent | ✅ 2026-06-10 (a6e9861) |
| 03 | [SubAgent 工具白名单](./03_SubAgent工具白名单.md) | L2 P0 | microclaw | ✅ 2026-06-10 (a6e9861) |
| 04 | [Skill 生命周期自动化](./04_Skill生命周期自动化.md) | L2 P0 | opencrust | ✅ 2026-06-10 (a6e9861) |
| 05 | [后台异步任务](./05_后台异步任务.md) | L2 P0 | oh-my-opencode | ✅ 2026-06-10 (a6e9861) |
| 06 | [压缩前 Memory Flush](./06_压缩前MemoryFlush.md) | L3 P1 ROI 高 | openclaw-mini | ⏳ 待启动 |
| 07 | [Hook 五层 Tier](./07_Hook五层Tier.md) | L3 P1 ROI 高 | oh-my-opencode | ⏳ 待启动 |
| 08 | [Hashline Edit 防 Stale](./08_HashlineEdit防Stale.md) | L3 P1 ROI 高 | oh-my-opencode | ⏳ 待启动 |
| 09 | [Context Window 五层优先](./09_ContextWindow五层优先.md) | L3 P1 ROI 高 | moltis | ⏳ 待启动 |
| 10 | [Session Queue 五种模式](./10_SessionQueue五种模式.md) | L3 P1 ROI 高 | octos | ⏳ 待启动 |
| 11 | [Verdict 四态与 BDD 验收](./11_Verdict四态与BDD验收.md) | L4 P1 ROI 中 | agent-spec | ⏳ 待启动 |
| 12 | [Provider 三层 Failover](./12_Provider三层Failover.md) | L4 P1 ROI 中 | octos | ✅ 2026-06-10 (ae6f7e3 v0.3) |
| 13 | [双层循环与 EventStream](./13_双层循环与EventStream.md) | L5 P1 投资大 | openclaw-mini | ⏳ 待启动 |
| 14 | [Knowledge 五状态与 Gate](./14_Knowledge五状态与Gate.md) | L5 P1 投资大 | mempal | ⏳ 待启动 |

## 跨缺口规范

不在本目录, 见 [`../规范/`](../规范/):

| 编号 | 名称 | 用途 |
|---|---|---|
| 15 | [文件层级设计](../规范/15_文件层级设计.md) | 14 缺口叠加后的目录布局 |
| 16 | [接口契约汇总](../规范/16_接口契约汇总.md) | 14 缺口核心接口签名 + 7 处一致性裁定 (**实施前必读**) |
| 17 | [异常路径](../规范/17_异常路径.md) | 14 缺口各自的失败场景 + 期望处理 |
| 18 | [可观测性规范](../规范/18_可观测性规范.md) | tracing 日志 / SseEvent / 指标埋点规范 |

## 不在本目录范围

- 14 缺口全景 + 实施顺序: [`../00_总览.md`](../00_总览.md)
- 已实现功能: `../../事实源/`
- 旧决策: `../../决策/`
- 阶段经验: `../../经验/`
- 跨 Track 规划: `../../子项目规划/`