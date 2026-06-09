# 千寻设计文档索引

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 目录

| 编号 | 文档 | 优先级 | 借鉴源 | 主题 |
|---|---|---|---|---|
| 00 | [00_总览.md](./00_总览.md) | — | — | 14 缺口的概览 + 实施顺序 |
| 01 | [01_Hook退出码与熔断.md](./01_Hook退出码与熔断.md) | 🔴 P0 | octos | HookResult::Error + circuit breaker |
| 02 | [02_LLM错误分类与恢复.md](./02_LLM错误分类与恢复.md) | 🔴 P0 | hermes-agent | FailoverReason 22 分类 + 决策树 |
| 03 | [03_SubAgent工具白名单.md](./03_SubAgent工具白名单.md) | 🔴 P0 | microclaw | sub-agent 9-tool 限制 |
| 04 | [04_Skill生命周期自动化.md](./04_Skill生命周期自动化.md) | 🔴 P0 | opencrust | 5+ 次自学习 + 30 天归档 |
| 05 | [05_后台异步任务.md](./05_后台异步任务.md) | 🔴 P0 | oh-my-opencode | background-task + 5 状态 |
| 06 | [06_压缩前MemoryFlush.md](./06_压缩前MemoryFlush.md) | 🟡 P1 | openclaw-mini | 压缩前无损落盘 |
| 07 | [07_双层循环与EventStream.md](./07_双层循环与EventStream.md) | 🟡 P1 | openclaw-mini | outer/inner + 20 事件 |
| 08 | [08_Provider三层Failover.md](./08_Provider三层Failover.md) | 🟡 P1 | octos | Retry/Chain/Router |
| 09 | [09_Hook五层Tier.md](./09_Hook五层Tier.md) | 🟡 P1 | oh-my-opencode | Session/ToolGuard/Transform/Continuation/Skill |
| 10 | [10_HashlineEdit防Stale.md](./10_HashlineEdit防Stale.md) | 🟡 P1 | oh-my-opencode | LINE#ID + hash 校验 |
| 11 | [11_Verdict四态与BDD验收.md](./11_Verdict四态与BDD验收.md) | 🟡 P1 | agent-spec | pass/fail/skip/uncertain |
| 12 | [12_ContextWindow五层优先.md](./12_ContextWindow五层优先.md) | 🟡 P1 | moltis | 5 层 precedence 链 |
| 13 | [13_Knowledge五状态与Gate.md](./13_Knowledge五状态与Gate.md) | 🟡 P1 | mempal | Draft→Canonical + Gate |
| 14 | [14_SessionQueue五种模式.md](./14_SessionQueue五种模式.md) | 🟡 P1 | octos | Followup/Collect/Steer/Interrupt/Speculative |
| 15 | [15_文件层级设计.md](./15_文件层级设计.md) | — | — | 14 缺口叠加后的文件/文件夹布局 |
| 16 | [16_接口契约汇总.md](./16_接口契约汇总.md) | — | — | 14 缺口的核心接口签名 + 7 处一致性裁定 |
| 19 | [19_异常路径.md](./19_异常路径.md) | — | — | 14 缺口各自的失败场景 + 期望处理 |
| 21 | [21_可观测性规范.md](./21_可观测性规范.md) | — | — | tracing 日志 / SseEvent / 指标埋点规范 |

## 引用方式

这些文档是 [agent_loop_v2.md](../10_事实源/架构/agent_loop_v2.md) 的**增量补齐**, 不替代它。

新会话 AI 读顺序:
1. [10_事实源/架构/agent_loop_v2.md](../10_事实源/架构/agent_loop_v2.md) — 基础设施 (Hook + 双轴 + SubAgent)
2. 本目录文档 — 在基础设施上叠加的能力, 按编号读
3. [00_总览.md](./00_总览.md) — 全景, 包含实施顺序
4. **实施前必读**:
   - [15_文件层级设计.md](./15_文件层级设计.md) — 14 缺口叠加后的目录布局
   - [16_接口契约汇总.md](./16_接口契约汇总.md) — 接口签名 + 7 处一致性裁定
   - [19_异常路径.md](./19_异常路径.md) — 失败场景
   - [21_可观测性规范.md](./21_可观测性规范.md) — 日志/事件规范

## 不在本目录范围

- 已实现功能: 见 `10_事实源/runtime-state.md` / `desktop-state.md`
- 旧决策: 见 `30_决策/`
- 阶段经验: 见 `40_经验/`
