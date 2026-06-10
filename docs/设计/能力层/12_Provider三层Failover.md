# 缺口 12: Provider 三层 Failover

> 状态: 已实现 v0.3 (Layer 1+2, Layer 3 留 P2) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-12 | 版本: v0.3
> **⚠️ 重建说明**: 本文档因 H4 修复脚本误操作 + git restore 删除后重建, 内容精简, 完整接口见 [`../规范/16_接口契约汇总.md`](../规范/16_接口契约汇总.md)。

## v0.3 实施记录 (2026-06-12)

**实施范围**: Layer 1 (RetryProvider, 同 provider 指数 backoff) + Layer 2 (ProviderChain, 切下个 provider).
**未实施**: Layer 3 (AdaptiveRouter + ProviderScoreboard) — 留 P2, 见 `../设计/规范/17_异常路径.md` §"Layer 3 决策".

### 关键决策

1. **ProviderStack 自己 `impl LlmProvider`** (forwarder 模式) — 业务调用方零修改, `processing_loop` / `SharedState` / `compact` 等全部继续用 `&dyn LlmProvider`, 实际拿到的是 `ProviderStack`.
2. **fallback 顺序不保证稳定** — `config.providers` 是 HashMap, 无序. `ProviderStack::new` 内部按 HashMap 迭代顺序拆 active + fallbacks. 升级稳定顺序时需改 `ResolvedConfig` 加 `provider_order: Vec<String>` 字段 (留 P2).
3. **Layer 3 缺失的 action (CompressContext / FallbackModel) 走 "break 返原始错误" 路径** — ProviderStack 看到这俩 action 当 terminal, 透传原始 LlmError 给上层.
4. **变体 A 风险缓解**: ProviderStack 只在 `stream_completion` 返 `Err` 时走 retry/rotate; `Ok(stream)` 直接透传. stream 已经开始后不切 provider, 避免双连接.
5. **`agent.retry_count` 字段从 AgentLoop 删除** — retry 决策已迁到 ProviderStack, 业务不再持有 per-call 重试状态. `AgentConfig.max_retries` 字段保留, ProviderStack 构造时读.

### 实施文档

详细实施记录 + 测试 + 行数统计: `../../经验/2026-06-12_缺口12_ProviderFailover_收尾.md`.

## 借鉴源

[octos](E:\giti\octos) Retry/Chain/AdaptiveRouter 三层 Failover。

## 问题

千寻当前**单 provider**, provider 挂 = 全 session 死。即使有 [缺口 02](./02_LLM错误分类与恢复.md) 的 LlmErrorKind, 也只能 retry 同一个 provider, 无 rotate 能力。

需要**三层 Failover** 提供 99.9% 可用性。

## 方案

### 三层架构

```rust
// qianxun-core/src/provider/failover.rs
pub struct ProviderStack {
    layer1_retry: RetryProvider,        // 5xx/429 → 3 次指数 backoff
    layer2_chain: ProviderChain,        // 401/403/超时 → 切下个 provider
    layer3_router: AdaptiveRouter,      // 长期不可用 → 评分系统重路由
}
```

### 评分系统

```rust
// qianxun-core/src/provider/scoreboard.rs
pub struct ProviderScoreboard {
    weights: HashMap<ProviderId, f32>,  // 0.0-1.0
    last_failure: HashMap<ProviderId, Instant>,
}
// 评分: 成功 +0.1, 失败 -0.3, 连续失败 → 熔断 5 分钟
```

## 跟其他缺口的关系

- 强依赖 [缺口 02](./02_LLM错误分类与恢复.md) 的 `LlmErrorKind`: RetryProvider 用它决定可重试集合
- 跟 [缺口 13](./13_双层循环与EventStream.md) 的 DualLoop 联动: 多次失败后 user prompt 走"切换 provider?" 询问

### 12.3 三层决策流程 (跟缺口 02 联动)

```
[ProviderStack::call()]
   │
   ├─ Layer 1: RetryProvider (同 provider, 指数 backoff)
   │  │
   │  ├─ 错误 → classify_by_lm_errorkind (缺口 02)
   │  │  ├─ Retryable (RateLimit, ServerError, Timeout)
   │  │  │  └─ backoff(attempt): 1s → 2s → 4s → 放弃
   │  │  ├─ CircuitBreak (Overloaded × 3)
   │  │  │  └─ 立即升级 Layer 2
   │  │  └─ Non-retryable (Auth, ContentPolicyBlocked, ModelNotFound)
   │  │     └─ 立即升级 Layer 2
   │  │
   │  └─ 3 次失败 → Layer 2
   │
   ├─ Layer 2: ProviderChain (切下一个 provider)
   │  │
   │  ├─ 遍历 providers[] 配置顺序 (deepseek → openai → minimax)
   │  │  跳过被 Layer 3 标记为 "Unavailable" 的 provider
   │  │
   │  └─ 全部失败 → Layer 3
   │
   └─ Layer 3: AdaptiveRouter (评分重路由)
      │
      ├─ 查 ProviderScoreboard, 选权重最高且 status=Available 的 provider
      │
      ├─ 调用失败 → 权重 -0.3, last_failure = now()
      │  连续失败 → 熔断 5 分钟 (status=Unavailable)
      │
      └─ 调用成功 → 权重 +0.1
```

**强依赖缺口 02**:
- `classify_by_lm_errorkind` 直接调 `LlmErrorKind::decide_recovery()`
- RetryProvider 用 `LlmErrorKind` 判断是否可重试 (`Retryable` vs `Non-retryable`)
- 评分系统收集 `LlmErrorKind::AllProvidersFailed` 时启动

**单测覆盖**:
- `test_layer1_retry_exhausted_then_escalate` — 3 次 Retry 后 Layer 2
- `test_layer2_chain_skip_unavailable` — 跳过熔断的 provider
- `test_layer3_scoreboard_downgrades_failing_provider`
- `test_all_three_layers_fail_returns_AllProvidersFailed`

## 文件改动

- `qianxun-core/src/provider/failover.rs` (新) ProviderStack + 3 层
- `qianxun-core/src/provider/scoreboard.rs` (新) ProviderScoreboard
- `qianxun-runtime/src/api/send.rs` 接入 stack
- `qianxun-core/src/provider/error.rs` 加 `AllProvidersFailed` 变体

**总计 ~510 行** (含三层集成 + 评分 + 熔断测试)

## 不做什么

- 不做多 region 跨地域 failover (单 region 多 provider 足够)
- 不做 user-level provider 选择 UI (放 v3+)
- 不做 cost-based routing (评分不含 cost 维度)

## 验收

- [ ] 三层 Failover 集成测试
- [ ] 评分 + 熔断 测试
- [ ] `cargo test -p qianxun-core -- provider::failover` 全过
