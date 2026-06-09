# 缺口 08: Provider 三层 Failover

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[octos](E:\git\ai\octos) 3-layer failover: `RetryProvider` (exponential backoff 429/5xx) → `ProviderChain` (顺序) → `AdaptiveRouter` (hedge racing + circuit breaker)。

## 问题

千寻 v2 `LlmProvider` 是单实例, provider 挂了 = 整个 session 死。

后果:
- DeepSeek 503 → 千寻用户全断
- Anthropic rate limit → 没自动切 minimax
- 没用 hedge racing, 慢 provider 拖累快 provider

## 方案

### 8.1 三层抽象

```rust
// qianxun-core/src/provider/failover.rs (新)

pub struct ProviderStack {
    layer1_retry: RetryProvider,           // 同 provider 重试 3 次, exp backoff
    layer2_chain: ProviderChain,            // 顺序切下一个 (provider 1 → 2 → 3)
    layer3_router: AdaptiveRouter,          // 并发 hedge racing
}

#[async_trait]
pub trait LlmProviderStack: Send + Sync {
    async fn call(&self, req: ChatRequest) -> Result<ChatStream, LlmError>;
}
```

### 8.2 RetryProvider (层 1)

```rust
pub struct RetryProvider {
    inner: Arc<dyn LlmProvider>,
    max_retries: u32,                    // 默认 3
    backoff: ExponentialBackoff,          // 100ms → 200 → 400 → ...
    retryable_kinds: HashSet<LlmErrorKind>,  // RateLimit, Overloaded, ServerError, Timeout
}
```

### 8.3 ProviderChain (层 2)

```rust
pub struct ProviderChain {
    providers: Vec<Arc<dyn LlmProvider>>,
}

impl ProviderChain {
    pub async fn call(&self, req: ChatRequest) -> Result<ChatStream, LlmError> {
        for provider in &self.providers {
            match provider.call(req.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(e) if is_transient(&e) => continue,  // 切下一个
                Err(e) => return Err(e),                 // 永久错误
            }
        }
        Err(LlmErrorKind::AllProvidersFailed)
    }
}
```

### 8.4 AdaptiveRouter (层 3, hedge racing)

```rust
pub struct AdaptiveRouter {
    providers: Vec<Arc<dyn LlmProvider>>,
    scoreboard: Arc<RwLock<HashMap<String, ProviderScore>>>,  // 滑动窗口评分
}

pub struct ProviderScore {
    success_rate: f32,        // 0.0 - 1.0
    avg_latency_ms: u32,
    circuit_open_until: Option<Instant>,
}

impl AdaptiveRouter {
    /// 同时发 N 个 provider, 第一个成功就用, 取消其他
    pub async fn call(&self, req: ChatRequest) -> Result<ChatStream, LlmError> {
        // 1. 选 top 2 (按 success_rate * (1 - latency_factor))
        // 2. 并发 race
        // 3. 第一个成功的用, 取消其他
        // 4. 更新 scoreboard
    }
}
```

### 8.5 配置

```json
// ~/.qianxun/config.json
{
  "provider_stack": {
    "retry": { "max_retries": 3, "backoff_ms": 100 },
    "chain": ["deepseek", "minimax", "anthropic"],
    "router": { "hedge_race_top_n": 2, "circuit_breaker_threshold": 5 }
  }
}
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/provider/failover.rs` (新) | 三层抽象 | +250 |
| `qianxun-core/src/provider/scoreboard.rs` (新) | 评分系统 | +80 |
| `qianxun-core/src/config.rs` | + provider_stack 配置 | +30 |
| `qianxun-runtime/src/api/send.rs` | 接入 stack | +30 |
| 测试 | 三层 + 评分 + 熔断 | +120 |

**总计 ~510 行**

## 不做什么

- 不做按模型复杂度自动选 (留给 openfang `ModelRoutingConfig`, 独立 PR)
- 不做跨 session 评分持久化 (内存, 重启丢)
- 不做 per-region 路由 (千寻单 region)

## 验收

- [ ] DeepSeek 503 → layer 1 重试 3 次 → 仍 fail → layer 2 切 minimax → 成功
- [ ] 慢 provider 拖累快 provider → hedge racing 选最快的
- [ ] 失败 5 次的 provider → circuit open, 60s 内不再尝试
- [ ] scoreboard 滑动窗口正确
