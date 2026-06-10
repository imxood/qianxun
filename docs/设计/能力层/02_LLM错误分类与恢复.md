# 缺口 02: LLM 错误分类与恢复

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-11 | 版本: v0.1
> **⚠️ 重建说明**: 本文档因 H4 修复脚本误操作 + git restore 删除后重建, 内容精简, 完整接口见 [`../规范/16_接口契约汇总.md`](../规范/16_接口契约汇总.md)。

## 借鉴源

[hermes-agent](E:\giti\hermes-agent) `error_classifier.py`: 22 种 `FailoverReason` + 决策树。

## 问题

千寻当前 `RuntimeApiError` 5 变体, 不区分 LLM 错误类型。所有错误统一返给前端, 用户看到"auth error"重试一次还是"auth error", 不会**自动切 provider / 压缩 context / backoff**。

正确处理需要 15+ 种分类 + 对应恢复动作。

## 方案

### 2.1 LlmErrorKind enum (15 个核心分类)

```rust
// qianxun-core/src/provider/error.rs
pub enum LlmErrorKind {
    Auth, AuthPermanent, Billing, RateLimit, Overloaded, ServerError,
    Timeout, ContextOverflow, PayloadTooLarge,
    ModelNotFound, ContentPolicyBlocked,
    FormatError, InvalidThinkingSig, Unknown,
    AllProvidersFailed,
}
```

### 2.2 RecoveryAction 决策

```rust
// qianxun-runtime/src/provider/recovery.rs
pub enum RecoveryAction {
    Retry { delay: Duration },
    RotateProvider,
    CompressContext,
    FallbackModel(String),
    Abort(String),
}
pub fn decide_recovery(kind: LlmErrorKind, ctx: &CallContext) -> RecoveryAction;
```

## 跟其他缺口的关系

- 提供 [缺口 12](./12_Provider三层Failover.md) 调用: 12 的 RetryProvider 用 `LlmErrorKind` 决定可重试集合
- 接入位置: `qianxun-runtime/src/api/send.rs` retry 循环

## 文件改动

- `qianxun-core/src/provider/error.rs` (新) LlmErrorKind + Classifier
- `qianxun-runtime/src/provider/recovery.rs` (新) decide_recovery
- `qianxun-runtime/src/api/send.rs` 接入 retry 循环

**总计 ~320 行** (含 15 分类单元测试 + 6 集成测试)

## 不做什么

- 不做 provider 评分系统 (留给 [缺口 12](./12_Provider三层Failover.md) 才做)
- 不做 fallback model 自动选择 (用户配置优先)

## 验收

- [ ] 15 种 LlmErrorKind 分类测试
- [ ] 401/429/500/503/timeout/context_overflow 各 1 集成测试
- [ ] `cargo test -p qianxun-core -- provider::error` 全过
