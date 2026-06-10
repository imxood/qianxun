# 缺口 12: Provider 三层 Failover

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-11 | 版本: v0.1
> **⚠️ 重建说明**: 本文档因 H4 修复脚本误操作 + git restore 删除后重建, 内容精简, 完整接口见 [`../规范/16_接口契约汇总.md`](../规范/16_接口契约汇总.md)。

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
