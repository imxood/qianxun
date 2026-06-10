# 缺口 10: Session Queue 五种模式

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[octos](E:\git\ai\octos) 5 queue modes per session: Followup / Collect / Steer / Interrupt / Speculative。

## 问题

千寻 v2 用户中途发消息**直接覆盖/排队**, 没模式选择:

- 用户追加指示 ("顺便也检查 X") → 当前 LLM 输出被覆盖
- 用户改方向 ("算了, 改成 Y") → 当前工具继续跑, 浪费 token
- 用户发多个相关问题 → 一股脑堆给 LLM, 上下文乱

## 方案

### 10.1 5 queue modes

```rust
// qianxun-runtime/src/queue.rs (新)

pub enum QueueMode {
    /// 当前跑完再处理 (默认)
    Followup,
    /// 收集一批一起处理 (等 2s 或 5 个)
    Collect,
    /// 中途可改方向, 注入 system prompt
    Steer,
    /// 立即中断当前
    Interrupt,
    /// 提前准备下一步 (提前 fetch 工具 / 预编译)
    Speculative,
}
```

### 10.2 SessionQueue 模块

```rust
pub struct SessionQueue {
    mode: Arc<RwLock<QueueMode>>,
    pending: Arc<Mutex<VecDeque<QueuedMessage>>>,
    collect_deadline: Arc<RwLock<Option<Instant>>>,
}

pub struct QueuedMessage {
    pub content: String,
    pub queued_at: Instant,
    pub source: MessageSource,  // UserInput / Steer / ToolResult
}

impl SessionQueue {
    pub async fn enqueue(&self, msg: QueuedMessage) -> EnqueueResult;
    pub async fn drain_pending(&self) -> Vec<QueuedMessage>;
    pub fn switch_mode(&self, mode: QueueMode);
}
```

### 10.3 5 mode 行为

| Mode | 入队时 | 处理时机 | 适用 |
|---|---|---|---|
| Followup | 直接入队 | 当前 turn 完成 | 默认 |
| Collect | 等 2s 或 5 个 | 2s/5 个后批量发 | 用户连发短问 |
| Steer | 立即入队 + 设标记 | 当前 tool call 后注入 | 用户改方向 |
| Interrupt | 设 cancel flag | 当前 turn 立即终止 | 用户想重做 |
| Speculative | 立即入队 | 工具执行时预跑下一步 | 预测性加速 |

### 10.4 接入 processing_loop_v2

```rust
// qianxun-core/src/processing_loop/v2.rs

async fn run(&self) {
    loop {
        // 1. 处理当前 turn
        self.execute_current_turn().await;

        // 2. 处理 queue (按 mode)
        match self.queue.mode().await {
            QueueMode::Followup => {
                if let Some(msg) = self.queue.pop().await {
                    self.inject_user_message(msg).await;
                }
            }
            QueueMode::Collect => {
                let msgs = self.queue.drain_after(2s).await;
                if !msgs.is_empty() { self.inject_batch(msgs).await; }
            }
            QueueMode::Steer => {
                while let Some(msg) = self.queue.pop_steer() {
                    self.inject_steer(msg).await;
                }
            }
            QueueMode::Interrupt => { break; }
            QueueMode::Speculative => { /* 预跑下一步 */ }
        }
    }
}
```

### 10.5 RuntimeApi + UI

```rust
// qianxun-runtime/src/api/trait_def.rs
async fn switch_queue_mode(&self, session_id: &str, mode: QueueMode) -> Result<()>;
```

Tauri 加 UI: session 设置面板 → 5 个 mode 单选按钮。

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-runtime/src/queue.rs` (新) | SessionQueue | +200 |
| `qianxun-core/src/processing_loop/v2.rs` | 接入 | +50 |
| `qianxun-runtime/src/api/trait_def.rs` | +1 方法 | +5 |
| `qianxun-desktop/src-tauri/src/commands/runtime/queue.rs` (新) | 1 command | +20 |
| `qianxun-desktop/src/lib/components/.../QueueModeSelect.svelte` (新) | UI | +60 |
| 测试 | 5 mode 行为 | +80 |

**总计 ~415 行**

## 不做什么

- 不做 mode 自动选择 (LLM 决策)
- 不做跨 session queue 共享
- 不做 queue 内容持久化 (重启丢)

## 验收

- [ ] Followup: 用户发 3 个 → 当前 turn 完成后, 3 个依次处理
- [ ] Collect: 用户发 3 个 (间隔 < 2s) → 2s 后批量发
- [ ] Steer: 用户发 "改方向" → 当前 tool 后立即注入
- [ ] Interrupt: 用户发 → 当前 turn 立即终止
- [ ] Speculative: 工具执行时后台预编译下一步
- [ ] 5 mode 运行时切换
