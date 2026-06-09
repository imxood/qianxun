# 缺口 06: 压缩前 Memory Flush

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[openclaw-mini](E:\git\ai\openclaw-mini) `compaction` 前的 `softThreshold` 提前 flush: 触发压缩前, 让模型把 durable info 落盘到 memory, 再做有损 compaction。

## 问题

千寻 `CompactZone` 直接压缩 conversation: 把旧消息摘成短摘要, 详细 tool_call 结果丢失。

长任务后果:
- 1 小时前查的关键事实 → 压缩后没了
- 用户的偏好 / 项目背景 → 压缩后没了
- 中间决策的 rationale → 压缩后没了

## 方案

### 6.1 Memory Flush 触发点

```rust
// qianxun-core/src/agent/context/compact.rs (改)

pub async fn maybe_compact(&self) -> CompactionResult {
    let usage = self.estimate_tokens();
    let soft_threshold = self.config.soft_threshold_ratio * self.config.max_tokens; // 0.75
    let hard_threshold = self.config.max_tokens; // 1.0

    if usage >= hard_threshold {
        // 先 flush, 再 compact
        self.flush_durable_to_memory().await?;
        self.compact_hard().await
    } else if usage >= soft_threshold {
        // 软触发, 只 flush 不 compact
        self.flush_durable_to_memory().await?;
        CompactionResult::Noop
    } else {
        CompactionResult::Noop
    }
}
```

### 6.2 flush_durable_to_memory 流程

```rust
async fn flush_durable_to_memory(&self) -> Result<()> {
    // 1. 拿最近 20 步的 message
    let recent = self.conversation.recent(20);

    // 2. 让 LLM 提取 "durable info" (跟 openclaw-mini 同):
    //    - 关键事实 (用户说的, 模型查到的)
    //    - 项目背景 / 用户偏好
    //    - 决策 rationale
    //    - 待办 / 下一步
    let prompt = format!("从以下对话中提取需要长期保留的信息:

{recent}

输出 JSON 数组 [{{category, content, importance}}]");
    let durable: Vec<DurableItem> = self.provider.call_json(prompt).await?;

    // 3. 写入 memory (跟 Reflect 联动, 缺口 13)
    for item in durable {
        self.memory.save(MemoryItem {
            category: item.category,
            content: item.content,
            importance: item.importance,
            source_session: self.session_id,
            created_at: now(),
        }).await?;
    }

    Ok(())
}
```

### 6.3 配置

```rust
// qianxun-core/src/config.rs

pub struct CompactionConfig {
    pub max_tokens: u32,                  // 128000
    pub soft_threshold_ratio: f32,        // 0.75
    pub hard_threshold_ratio: f32,        // 1.0
    pub flush_enabled: bool,              // 默认 true
    pub flush_recent_steps: u32,          // 默认 20
}
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/agent/context/compact.rs` | + flush_durable | +80 |
| `qianxun-core/src/config.rs` | +3 配置字段 | +10 |
| `qianxun-memory` (已存在) | save 新接口 | +20 |
| 测试 | flush 触发 + durable 正确 | +50 |

**总计 ~160 行**

## 不做什么

- 不做"哪些信息重要"的 LLM 训练 — 用 prompt 模板 + 简单启发式
- 不做 memory 摘要压缩 — 留给缺口 13
- 不做 per-user 重要性阈值 — 全局配置

## 验收

- [ ] 0.75 阈值时 → flush, 不 compact
- [ ] 1.0 阈值时 → flush + compact
- [ ] flush 后 30 分钟对话的关键事实能在 memory 检索到
- [ ] flush 失败不阻塞 compact (fallback)
