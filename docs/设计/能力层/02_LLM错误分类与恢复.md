# 缺口 02: LLM 错误分类与恢复

> 状态: 已实现 (v0.2, 33 单测 passed) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-11 | 版本: v0.2
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

### 2.3 LlmErrorKind → RecoveryAction 决策表

| LlmErrorKind | 默认 RecoveryAction | 备注 |
|---|---|---|
| `Auth` / `AuthPermanent` / `Billing` | `Abort("需要人工换 key")` | 不可重试, 立即中止 |
| `RateLimit` | `Retry { delay: exponential_backoff(retry_count) }` | 3 次后切 [缺口 12](./12_Provider三层Failover.md) layer2 |
| `Overloaded` | `RotateProvider` | 立即切下一个 provider |
| `ServerError` | `Retry { delay: 1s }` | 5xx 通常短暂, retry 即可 |
| `Timeout` | `Retry { delay: 2s }` | 重试 1 次后降级 |
| `ContextOverflow` | `CompressContext` | 触发 [缺口 09](./09_ContextWindow五层管理.md) |
| `PayloadTooLarge` | `CompressContext + FallbackModel(mini)` | 同上 |
| `ModelNotFound` | `FallbackModel(default)` | 切兜底模型 |
| `ContentPolicyBlocked` | `Abort("内容违规")` | 不可重试 |
| `FormatError` / `InvalidThinkingSig` | `Retry { delay: 500ms }` | 重试一次, 第二次 Abort |
| `AllProvidersFailed` | `Abort("所有 provider 都不可用")` | 顶层 catch-all |
| `Unknown` | `Retry { delay: 1s }` | 未知分类保守重试一次 |

### 2.4 HTTP 错误码 → LlmErrorKind 映射

| HTTP Status | LlmErrorKind | 触发场景 |
|---|---|---|
| 401 / 403 | `Auth` / `AuthPermanent` | API key 无效或过期 |
| 402 | `Billing` | 余额不足 |
| 429 | `RateLimit` | 触发限流 |
| 500 / 502 / 503 / 504 | `ServerError` / `Overloaded` | 内部错误或临时过载 |
| 408 | `Timeout` | 请求超时 |
| 413 | `PayloadTooLarge` | 请求体超限 |
| 400 (含 `context_length_exceeded`) | `ContextOverflow` | Context 超限 |
| 400 (含 `model_not_found`) | `ModelNotFound` | 模型不存在 |
| 400 (含 `content_policy`) | `ContentPolicyBlocked` | 内容违规 |
| 4xx 其他 | `FormatError` | 解析失败 |
| 5xx 其他 | `Unknown` | 未分类 |

## 跟其他缺口的关系

- 提供 [缺口 12](./12_Provider三层Failover.md) 调用: 12 的 RetryProvider 用 `LlmErrorKind` 决定可重试集合
- 接入位置: `qianxun-runtime/src/api/send.rs` retry 循环

## 跟 SseEvent 联动

流末尾 `LlmErrorKind` 通过 `SseEvent::Error` 变体上报到客户端:

```rust
// qianxun-runtime/src/api/sse_event.rs
pub enum SseEvent {
    // ... 现有 12 变体 ...
    Error(LlmErrorKind),  // 新增: 缺口 02 引入
}
```

事件字段 (序列化):

| 字段 | 类型 | 说明 |
|---|---|---|
| `kind` | string | `LlmErrorKind` 枚举值 (snake_case) |
| `message` | string | 原始错误描述 (用户可见) |
| `recovery` | string | 已采取的 `RecoveryAction` (e.g. `rotate_provider`, `retry`) |

客户端处理 (Svelte 5):
- 收到 `Error` 事件 → `chat.svelte.ts` `state.error = kind`
- 显示对应 toast (e.g. `RateLimit - 正在重试 (1/3)`)
- `Abort` 类型 → 终止流, 提示用户操作 (e.g. `Auth - 请检查 API key`)
- `e2e/test_daemon.py::test_002_llm_error_propagation` 验证 401 / 429 / 500 三种 SSE 错误传播

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
