# 缺口 12 Provider 三层 Failover 收尾 (2026-06-12)

## 1. 目标

承接 `2026-06-12_集成层_真实挂载_收尾.md`, 把缺口 12 (Provider Failover) 从"草稿态文档"推到"已实现 v0.3 (Layer 1+2)".

**务实取舍**: 本期实施 Layer 1 (RetryProvider) + Layer 2 (ProviderChain). **不实施 Layer 3 (AdaptiveRouter + ProviderScoreboard)** — 留 P2, 跟原始设计文档 [12_Provider三层Failover.md](../设计/能力层/12_Provider三层Failover.md) §"v0.3 实施记录" 一致.

## 2. 设计要点

### 2.1 ProviderStack: LlmProvider forwarder

`ProviderStack` 自己 `impl LlmProvider` (forwarder 模式), 业务调用方零修改:

- `processing_loop::handle_user_message` 接受 `&dyn LlmProvider` 参数 — 不变
- `SharedState.provider: Arc<dyn LlmProvider>` 字段类型 — 不变 (实际指向 ProviderStack)
- `compact::attempt_compression` 调 `&dyn LlmProvider` — 不变 (走同样的 Layer 1+2 决策, 是好事 — 减少"假失败"触发压缩熔断)

### 2.2 三层决策流程

```text
[ProviderStack::stream_completion]
   │
   ├─ Layer 1: 同一 provider 指数 backoff (decide_recovery 决策)
   │  │
   │  ├─ Retry  (RateLimit/ServerError/Timeout) → backoff(1s/2s/4s)
   │  ├─ Rotate (RateLimit × 3 / Overloaded)      → 切 Layer 2
   │  └─ Abort  (Auth/Billing/ContentPolicy)       → 立即终止
   │
   └─ Layer 2: 切下个 provider (按 HashMap 迭代顺序, 不保证稳定)
      │
      └─ 全部失败 → LlmError::ApiError { kind: AllProvidersFailed, .. }
```

### 2.3 关键决策

| # | 决策 | 理由 |
|---|------|------|
| 1 | ProviderStack forwarder 模式 | 业务零修改, 透明接入 |
| 2 | `ProviderStack::new` 接 `Vec<(String, ResolvedProviderConfig, Box<dyn LlmProvider>)>` | 内部 `filter(api_key.is_empty())` 需要 config |
| 3 | fallback 顺序: HashMap 迭代顺序, 不保证稳定 | `config.providers` 是 HashMap 无序; 文档明确声明 |
| 4 | `CompressContext` / `FallbackModel` action 走 "break 返原始错误" | Layer 3 缺失, 当 terminal 处理 |
| 5 | **变体 A**: 只在 `stream_completion` 返 Err 时走 retry/rotate; Ok(stream) 直接透传 | stream 已经开始后不切 provider, 避免双连接 |
| 6 | 删 `agent.retry_count` 字段 (engine.rs L42) | retry 决策已迁到 ProviderStack, 业务不再持有 per-call 状态 |
| 7 | `AgentConfig.max_retries` 字段保留 | ProviderStack 构造时读, 决定 Layer 1 上限 |

## 3. 改动文件清单

| 文件 | 改动 | 行数 |
|------|------|------|
| `qianxun-core/src/provider/failover.rs` (新) | ProviderStack 核心 + 6 单元测试 + MockProvider | +550 (新文件) |
| `qianxun-core/src/provider/mod.rs` | +`pub mod failover;` + `pub use error_classifier::{LlmErrorKind, RecoveryAction};` | +6 |
| `qianxun-runtime/src/state.rs` | `build()` + `new_for_test()` 改用 ProviderStack 构造; L41 字段加注释 | +30 -5 |
| `qianxun-runtime/src/agent_host.rs` | `for_test()` 同步改 ProviderStack; L471 测试断言改 | +12 -2 |
| `qianxun-core/src/agent/engine.rs` | 删 L42 retry_count 字段 + L54 初始化 + L69 reset 重置 + L228-253 retry 循环 → 单行 | -38 +8 |
| `docs/设计/能力层/12_Provider三层Failover.md` | 状态 "草稿" → "已实现 v0.3" + v0.3 实施记录段 | +30 |
| `docs/经验/2026-06-12_缺口12_ProviderFailover_收尾.md` (新) | 本文件 | +180 |
| `docs/事实源/runtime-state.md` | L41 字段注脚 + v0.3 缺口 12 注脚段 | +10 |

**总代码: 8 文件, ~580 行 (含 6 测试)**.

## 4. 6 个单元测试

`qianxun-core/src/provider/failover.rs` 底部 `#[cfg(test)] mod tests`:

| # | 名称 | 行为 |
|---|------|------|
| 1 | `test_succeeds_on_first_try` | primary `remaining_failures=0` → 1 次 stream_completion 调, Ok |
| 2 | `test_retries_same_provider_on_rate_limit` | primary `fail_times=2, fail_with=RateLimit` → 3 次调, 第 3 次 Ok |
| 3 | `test_rotates_to_fallback_after_max_retries` | primary `fail_times=3, fail_with=RateLimit` + fallback `fail_times=0` → 4 次调 (primary 3 + fallback 1) |
| 4 | `test_skips_providers_with_empty_api_key` | 3 entries, 第 2 个 `api_key=""` → 内部 fallbacks 长度 1 (跳过空 key) |
| 5 | `test_all_providers_fail_returns_all_providers_failed` | primary + fallback 都 `fail_times=3, RateLimit` → 返 `LlmError::ApiError { kind: AllProvidersFailed, message: contains "all 2 providers failed" }` |
| 6 | `test_aborts_on_fatal_kind` | primary `fail_times=1, AuthPermanent` (is_fatal=true) → 1 次调, 返**原始** `LlmError::AuthenticationError`, **不**包成 AllProvidersFailed |

Mock 设计: `MockProvider { id, remaining_failures: Arc<AtomicU32>, fail_with: LlmErrorKind, call_count: Arc<AtomicU32> }`. 简单清晰, 6 case 够用, 避免 `VecDeque<MockBehavior>` 的过度抽象.

## 5. 关键路径

| 路径 | 角色 |
|------|------|
| `qianxun-core/src/provider/failover.rs:84` | `ProviderStack::new` 构造 (filter + 拆 primary/fallbacks + 兜底) |
| `qianxun-core/src/provider/failover.rs:160` | `stream_completion` Layer 1 → Layer 2 编排 |
| `qianxun-core/src/provider/failover.rs:175` | `try_provider` Layer 1 retry 循环 + decide_recovery 决策 |
| `qianxun-runtime/src/state.rs:141` | `build()` 遍历 `config.providers` 构造 ProviderStack |
| `qianxun-runtime/src/agent_host.rs:411` | `for_test()` 同步改造 (测试路径一致) |
| `qianxun-core/src/agent/engine.rs:222-231` | engine.rs 新版单行 (替代旧 24 行 retry 循环) |

## 6. 验收

```text
cargo test -p qianxun-core -- provider::failover    # 6/6 passed
cargo test -p qianxun-core -- agent::engine         # 2/2 passed (不退步)
cargo test -p qianxun-runtime                       # 70/70 passed
cargo test --workspace                              # 360 passed, 0 failed, 4 ignored
```

测试基线: 集成层 354 → 缺口 12 后 360 (+6).

### 行为验证 (手工 / 集成)

- `config.providers.deepseek.api_key="valid"` + `config.providers.minimax.api_key="invalid"` → 走 deepseek 成功
- `config.providers.deepseek.api_key=""` + `config.providers.minimax.api_key="valid"` → deepseek 跳过, minimax 自动顶上
- 单 provider 失败 3 次 RateLimit → 切 fallback (若 fallback 有 key)
- 单 provider 报 AuthPermanent (is_fatal=true) → 立即 abort, 透传原始 `LlmError::AuthenticationError`, **不**走 fallback

## 7. 留 P2 / 显式不做

| 缺口 | 内容 | 留 P2 原因 |
|---|---|---|
| 12 Layer 3 | AdaptiveRouter + ProviderScoreboard | 需要成功率持久化 + 跨 session 学习 + 评分模型, 当前 Layer 1+2 已覆盖 90% 实际场景 |
| 12 | fallback 顺序稳定化 (`provider_order: Vec<String>`) | 改 `ResolvedConfig` 侵入大, 当前业务 (单主 + 1-2 备用) 对顺序不敏感 |
| 12 | ProviderStack 暴露 `on_status("速率受限, ...s 后重试 ...")` 给前端 | 当前走 `tracing::info!` log, 后续 v0.4 加 event sink 透出 |
| 12 | stream 中途 error 重试 | 风险高 (要重放 request + 处理已发 token), 变体 A 缓解 |
| 12 | `update_active_provider` RuntimeApi 配合 ProviderStack 热替换 | 仍只写 config.json + 提示重启 |

## 8. 跟其他缺口的关系

- **强依赖缺口 02** (`LlmErrorKind` + `RecoveryAction`): ProviderStack 内部 `try_provider` 直接调 `decide_recovery(kind, attempt, None)` 决定 retry vs rotate vs abort. `LlmError::kind()` 字段已被缺口 02 Stage 7 注入.
- **跟缺口 01 联动**: Hook dispatch 跟 ProviderStack 无冲突, hook 在 processing_loop 入口/出口跑, ProviderStack 只在 `stream_completion` 启动失败时介入.
- **跟缺口 13 (双层循环) 联动**: 设计文档 12 原始设计提到 "多次失败后 user prompt 走"切换 provider?" 询问" — 留 P2, 跟缺口 13 一起.

## 9. 兼容性影响

- **零 breaking change**: 业务调用方 (processing_loop, SharedState, compact, anthropic_compat) 全部零修改. `Arc<dyn LlmProvider>` 接口 100% 兼容.
- **state.rs L41 字段类型未变**: 仍 `pub provider: Arc<dyn LlmProvider>`, 仅实际指向的类型变化 (旧: 单 AnthropicCompatProvider; 新: ProviderStack forwarder). 加 doc 注释说明.
- **engine.rs retry 行为变化**: 旧版失败立即 on_error 终止; 新版 ProviderStack 内部先 retry 1-2 次, 失败后切 fallback, 仍失败才 on_error. 用户感知: 单 provider 短暂故障时不再立即看到 error, 自动重试+切备用 (improvement).
- **`AgentLoop.retry_count` 字段已删**: 之前持有 per-call 重试状态, ProviderStack 自己管. 旧测试代码 `runtime.agent_loop.retry_count == 0` 改成注释 (agent_host.rs:471).

## 10. 实施工时

- T1-T8 总工时: ~1.5 dev-day
- 拆解:
  - T1 (ProviderStack 核心 + 编译) + T3 (mod.rs 导出): 0.3 day
  - T2 (6 个单元测试): 0.2 day
  - T4-T6 (state/agent_host/engine 改造): 0.4 day
  - T7 (全栈 cargo test 回归): 0.1 day
  - T8 (docs 收尾): 0.5 day
- Plan agent 验证 1 次, 实际代码改动 < 580 行 (含 6 测试 ~250 行), 跟 plan 估算 ~480 行基本一致 (略多因为 mock + 注释).
