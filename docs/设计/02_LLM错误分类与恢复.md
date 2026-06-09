# 缺口 02: LLM 错误分类与恢复

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[hermes-agent](E:\git\ai\hermes-agent) `error_classifier.py`: 22 种 `FailoverReason` + 决策树。

## 问题

千寻当前 `RuntimeApiError` 5 变体, 不区分 LLM 错误类型。所有错误统一返给前端, 用户看到"auth error"重试一次还是"auth error", 不会**自动切 provider / 压缩 context / backoff**。

正确处理需要 22 种分类 + 对应恢复动作。

## 方案

### 2.1 LlmErrorKind enum (15 个核心分类)

```rust
// qianxun-core/src/provider/error.rs

pub enum LlmErrorKind {
    // 鉴权
    Auth,                  // 401/403 → refresh/rotate
    AuthPermanent,         // refresh 后仍 401 → abort

    // 配额
    Billing,               // 402 → rotate immediately
    RateLimit,             // 429 → backoff + rotate

    // 服务端
    Overloaded,            // 503/529 → backoff
    ServerError,           // 500/502 → retry

    // 传输
    Timeout,               // 网络 → rebuild + retry

    // Context
    ContextOverflow,       // 触发 compress, 不 failover
    PayloadTooLarge,       // 413 → compress

    // Provider 政策
    ModelNotFound,         // 404 → fallback model
    ContentPolicyBlocked,  // 安全拦截 → 确定性 per-request, 不重试

    // 协议
    FormatError,           // 400 → abort
    InvalidThinkingSig,    // Anthropic thinking block 失效 → strip + retry

    // 兜底
    Unknown,               // retry with backoff
}
```

### 2.2 错误分类器

```rust
pub struct LlmErrorClassifier;

impl LlmErrorClassifier {
    /// 从 reqwest 错误 + response body 推断 kind
    pub fn classify(status: Option<u16>, body: &str, transport: &TransportError) -> LlmErrorKind;
}
```

### 2.3 恢复决策树

```rust
// qianxun-runtime/src/provider/recovery.rs

pub enum RecoveryAction {
    Retry { delay: Duration },
    RotateProvider,            // 切下一个 provider (缺口 08 联动)
    CompressContext,           // 触发 compaction
    FallbackModel(String),     // 切下一个 model
    Abort(String),             // 致命错误
}

pub fn decide_recovery(kind: LlmErrorKind, context: &CallContext) -> RecoveryAction;
```

### 2.4 接入 send_message 循环

```rust
// qianxun-runtime/src/api/send.rs

for attempt in 1..=max_retries {
    match provider.call(req).await {
        Ok(stream) => return stream,
        Err(e) => {
            let kind = LlmErrorClassifier::classify(e.status, e.body, e.transport);
            let action = decide_recovery(kind, &ctx);
            match action {
                RecoveryAction::Retry { delay } => tokio::time::sleep(delay).await,
                RecoveryAction::RotateProvider => {
                    ctx.rotate_provider();  // 触发缺口 08
                    continue;
                }
                RecoveryAction::CompressContext => {
                    ctx.compress_conversation().await?;
                    continue;
                }
                RecoveryAction::FallbackModel(m) => {
                    ctx.set_model(m);
                    continue;
                }
                RecoveryAction::Abort(msg) => return Err(msg),
            }
        }
    }
}
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/provider/error.rs` (新) | LlmErrorKind + Classifier | +150 |
| `qianxun-runtime/src/provider/recovery.rs` (新) | decide_recovery | +60 |
| `qianxun-runtime/src/api/send.rs` | 接入 retry 循环 | +30 |
| 测试 | 22 种分类 case | +80 |

**总计 ~320 行** (含测试)

## 不做什么

- 不做模型自动降级 (用户配置 model, 不擅自改)
- 不做 provider 评分系统 (缺口 08 才做)
- 不做 per-error 监控上报 (留 telemetry 阶段)

## 验收

- [ ] 401 → refresh → 仍 401 → Abort
- [ ] 429 → backoff → 第二次 200 → 继续
- [ ] ContextOverflow → compress → 重试 → 200
- [ ] 500 → retry 3 次 → 仍 500 → 切下一个 provider
- [ ] 503 → backoff 5s → 第二次 200
