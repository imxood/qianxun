# 缺口 01: Hook 退出码与熔断

> 状态: 已实现 (v0.2, 12 单测 passed) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-11 | 版本: v0.2

## 借鉴源

[octos](E:\git\ai\octos) hooks 系统: 退出码语义 + 3 连败 circuit breaker。

## 问题

千寻 v2 `HookResult` 只有 4 变体: `Continue / Block / Modify / Terminate`, 缺 `Error`。

当 hook 自身 panic / 超时 / 抛异常时, 当前会**静默吞掉**, 继续走主循环。后果:
- hook 反复故障时, 主循环卡顿
- 无任何告警, 用户不知道 hook 在 fail
- 一旦 hook 持续失败, 整 session 退化

## 方案

### 1.1 HookResult 加 Error 变体

```rust
// qianxun-core/src/hooks/handler.rs

pub enum HookResult {
    Continue,
    Block(String),
    Modify(serde_json::Value),
    ForkSubAgent(SubAgentSpec),
    Terminate,
    Error(String),  // 新增: hook 自身异常
}
```

### 1.2 退出码语义 (octos 同源)

| HookResult | 退出码等价 | 含义 | 处理 |
|---|---|---|---|
| Continue | 0 | 允许 | 继续循环 |
| Block | 1 | 拒绝 | 不调工具, 记日志 |
| Modify | 0 | 允许 (改后) | 用新 args 调工具 |
| ForkSubAgent | 0 | 允许 (fork) | 主 + sub 并行 |
| Terminate | 0 | 主动结束 | 收尾 |
| **Error** | 2 | **hook 异常** | **记错误, 触发熔断计数** |

### 1.3 HookRegistry 加熔断

```rust
// qianxun-core/src/hooks/registry.rs

pub struct HookStats {
    name: String,
    consecutive_failures: AtomicU32,
    total_invocations: AtomicU64,
    total_failures: AtomicU64,
    disabled_at: Option<Instant>,  // 熔断触发时间
}

pub struct HookRegistry {
    // ... 4 槽位 ...
    stats: DashMap<String, Arc<HookStats>>,
    circuit_breaker_threshold: u32,  // 默认 3
    circuit_breaker_cooldown: Duration,  // 默认 60s
}

impl HookRegistry {
    pub async fn dispatch(&self, event: HookEvent<'_>) -> HookResult {
        for handler in self.handlers_for(&event) {
            if let Some(stats) = self.stats.get(handler.name()) {
                if stats.is_disabled() {
                    tracing::warn!(hook = handler.name(), "hook disabled, skip");
                    continue;
                }
            }
            let result = handler.handle(ctx).await;
            self.record_result(handler.name(), &result);
            if let HookResult::Error(_) = &result {
                if self.should_trip(handler.name()) {
                    self.disable(handler.name());
                    let _ = self.event_tx.send(HookEvent::HookDisabled { ... });
                }
            }
            // 短路语义: Block / Terminate / Error 立即返回
            if !matches!(result, HookResult::Continue) { return result; }
        }
        HookResult::Continue
    }
}
```

### 1.4 新增 SseEvent

```rust
// qianxun-runtime/src/sse.rs

HookDisabled { session_id, hook_name, cooldown_sec },
HookRecovered { session_id, hook_name },
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/hooks/handler.rs` | HookResult::Error | +5 |
| `qianxun-core/src/hooks/registry.rs` | HookStats + 熔断逻辑 | +60 |
| `qianxun-runtime/src/sse.rs` | +2 SseEvent 变体 | +10 |
| 测试 | 熔断 + 恢复 | +30 |

**总计 ~105 行** (包含测试)

## 不做什么

- 不做自动恢复探测 (冷却 60s 后只允许 1 次, 失败再 disable)
- 不做 hook 健康检查主动 ping
- 不做 per-hook 配置 (threshold 全局统一 3 次)

## 验收

- [ ] 单 hook 连续 3 次 Error → 第 4 次不调用 + 触发 SseEvent::HookDisabled
- [ ] 冷却 60s 后 hook 自动 re-enable
- [ ] 多个 hook 独立计数, 一个挂不影响其他
