# 缺口 06: 压缩前 Memory Flush

> 状态: 待 code review | 适用范围: qianxun-core / qianxun-memory | 最后更新: 2026-06-11 | 版本: v0.2
> **⚠️ 重建说明**: 本文档因 H4 修复脚本误操作 + git restore 删除后重建, 内容精简, 完整接口见 [`../规范/16_接口契约汇总.md`](../规范/16_接口契约汇总.md)。

## 借鉴源

[openclaw-mini](E:\giti\openclaw-mini) `softThreshold`: 压缩前先 flush 关键信息到 memory, 避免压缩丢上下文。

## 问题

千寻 `CompactZone` 直接压缩 conversation, 长任务里用户提到的关键信息 (e.g. "记住我接下来都用深色模式") 会**直接被压缩丢弃**。

需要在压缩前**主动 flush 关键信息到 memory**。

## 方案

```rust
// qianxun-core/src/agent/context/compact.rs
impl CompactZone {
    pub async fn maybe_compact(&self) -> CompactionResult;  // 改造: 加 flush
    pub async fn flush_durable_to_memory(&self) -> Result<Vec<KnowledgeItemId>>;  // 新增
}
```

**两层阈值**:
- soft 阈值 (e.g. 70%): 触发 flush
- hard 阈值 (e.g. 90%): 触发 compress

## 跟其他缺口的关系

- flush 调 `memory.save()` (status=Draft), 异步触发 [缺口 14](./14_Knowledge五状态与Gate.md) 的 `evaluate_promotion`
- 跟 Reflect 联动: flush 后 reflect hook 可检查 promote

## flush 时序 (跟缺口 14 联动)

```
[CompactZone::maybe_compact()]
   │
   ├─ context_token_usage >= soft_threshold (70%)
   │  └─ flush_durable_to_memory()
   │     │
   │     ├─ 1. 扫描 messages, 提取"用户偏好/事实/约定"三类
   │     │     (用 prompt_template `EXTRACT_KNOWLEDGE`)
   │     │
   │     ├─ 2. 对每条知识调用 memory.save(status=Draft)
   │     │     └─ memory row: (content, status=Draft, source_session_id)
   │     │
   │     └─ 3. 异步触发 evaluate_promotion (缺口 14)
   │           ├─ 引用次数 ≥ 5 → Candidate
   │           ├─ Candidate + 无 counterexample → Promoted
   │           └─ 3+ 引用 + 长期稳定 → Canonical
   │
   └─ context_token_usage >= hard_threshold (90%)
      └─ compact() 真正压缩 conversation
         (此时已被 flush 的知识已在 memory, 不会丢失)
```

**关键设计**: flush 在 compress **之前**, 保证压缩不丢用户偏好。

**软/硬阈值配置** (config.rs):

```rust
pub struct CompactConfig {
    pub soft_threshold: f32,    // 默认 0.7
    pub hard_threshold: f32,    // 默认 0.9
    pub flush_interval_secs: u64,  // 默认 60 (避免短时间内重复 flush)
}
```

**单测覆盖**:
- `test_soft_threshold_triggers_flush` — context 70% 时 flush 不 compress
- `test_hard_threshold_triggers_compress` — context 90% 时 compress (flush 已先跑过)
- `test_flush_no_duplicate_within_interval` — 60s 内不重复 flush
- `test_flush_then_compact_preserves_knowledge` — 压缩后 memory 仍能查到

## 文件改动

- `qianxun-core/src/agent/context/compact.rs` 加 `flush_durable_to_memory`
- `qianxun-core/src/config.rs` 加 3 配置字段 (soft/hard 阈值 + flush 间隔)
- `qianxun-memory` (已存在) save 新接口

**总计 ~160 行** (含 soft/hard 阈值 + flush 命中测试)

## 不做什么

- 不做 memory 摘要压缩 — 留给 [缺口 13](./13_双层循环与EventStream.md)
- 不做自动 promote — 留给 [缺口 14](./14_Knowledge五状态与Gate.md) 的 gate

## 验收

- [ ] soft/hard 阈值触发测试
- [ ] flush 后 memory 查询能命中
- [ ] `cargo test -p qianxun-core -- agent::context::compact` 全过
