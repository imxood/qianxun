# 缺口 07: 双层循环与 EventStream

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[openclaw-mini](E:\git\ai\openclaw-mini) 双层循环: outer (follow-up) + inner (tool+steer) + 20 种 EventStream 事件。

## 问题

千寻 v2 单循环: 用户中途发消息会**打断当前 turn**, 当前 LLM 输出和工具执行被中断。

后果:
- 长工具调用中用户不能追加指示
- steer 消息 (用户改方向) 没法插队
- 多 turn 间无显式状态转换事件

## 方案

### 7.1 双层循环

```rust
// qianxun-core/src/processing_loop/dual.rs (新)

pub struct DualLoop {
    inner_tx: mpsc::Sender<InnerEvent>,   // inner → outer
    outer_tx: mpsc::Sender<OuterEvent>,   // outer → inner
    user_input: mpsc::Receiver<UserMessage>,  // 注入用户输入
    steer: mpsc::Receiver<SteerMessage>,  // 注入中途改向
}

impl DualLoop {
    pub async fn outer_loop(&self) {
        // 处理 user_input queue, 起 inner_loop
        while let Some(msg) = self.user_input.recv().await {
            let inner_handle = self.start_inner_loop(msg).await;
            // 等 inner 完成, 但允许用户追加 steer
            tokio::select! {
                _ = inner_handle => {},
                steer = self.steer.recv() => {
                    self.outer_tx.send(OuterEvent::Steer(steer)).await;
                }
            }
        }
    }

    pub async fn inner_loop(&self, msg: UserMessage) {
        // 处理 tool + steer + 步间事件
        loop {
            tokio::select! {
                tool_result = self.execute_next_tool() => {
                    self.emit(InnerEvent::ToolDone(tool_result));
                }
                steer = self.steer.recv() => {
                    // 立即应用 steer 到当前 prompt
                    self.apply_steer(steer);
                    self.emit(InnerEvent::SteerApplied);
                }
                _ = self.check_budget() => {
                    self.emit(InnerEvent::BudgetExhausted);
                    break;
                }
            }
        }
    }
}
```

### 7.2 20 种 EventStream 事件

```rust
// qianxun-core/src/processing_loop/event.rs (新)

pub enum AgentEvent {
    // Outer 层 (6)
    UserInput { session_id, content },
    InnerStarted { session_id, turn_id },
    InnerCompleted { session_id, turn_id, result },
    InnerFailed { session_id, turn_id, error },
    SteerApplied { session_id, steer },
    SessionPaused { session_id },

    // Inner 层 (10)
    LlmRequestStarted { turn_id, prompt_tokens },
    LlmChunk { turn_id, delta },
    LlmRequestCompleted { turn_id, output_tokens, latency_ms },
    ToolCallStarted { turn_id, tool_name, args },
    ToolCallCompleted { turn_id, tool_name, result },
    ToolCallFailed { turn_id, tool_name, error },
    CompactionTriggered { turn_id, kind },
    MemoryFlushed { turn_id, items_saved },
    SubAgentSpawned { subagent_id, task },
    SubAgentCompleted { subagent_id, result },

    // 内部 (4)
    LoopIter { turn_id, iter },
    BudgetCheck { turn_id, remaining },
    QueueModeChanged { session_id, mode },
    BackoffSleeping { provider, duration_ms },
}
```

### 7.3 RuntimeApi 加订阅

```rust
// qianxun-runtime/src/api/trait_def.rs

async fn subscribe_events(&self, session_id: &str) -> Receiver<AgentEvent>;
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/processing_loop/dual.rs` (新) | 双层循环 | +180 |
| `qianxun-core/src/processing_loop/event.rs` (新) | 20 事件 | +80 |
| `qianxun-core/src/processing_loop/mod.rs` | 接入 v2 | +20 |
| `qianxun-runtime/src/api/trait_def.rs` | +1 订阅方法 | +5 |
| 测试 | 双层 + 事件顺序 | +80 |

**总计 ~365 行**

## 不做什么

- 不做 steer 的 LLM-based 改写 (直接注入 system prompt)
- 不做事件历史持久化 (实时推送, 不存盘)
- 不做事件 filter (前端按需订阅)

## 验收

- [ ] 工具执行中用户发 steer → 立即应用 + 继续
- [ ] 工具执行中用户发新 input → 等 inner 完成后起新 turn
- [ ] 20 事件按顺序触发
- [ ] session 暂停时事件队列不丢
