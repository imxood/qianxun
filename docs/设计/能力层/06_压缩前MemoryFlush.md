# 缺口 06: 压缩前 Memory Flush

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-memory | 最后更新: 2026-06-11 | 版本: v0.1
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
