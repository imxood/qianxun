# 缺口 14: Knowledge 五状态与 Gate

> 状态: 草稿 (待 code review) | 适用范围: qianxun-memory / qianxun-core | 最后更新: 2026-06-11 | 版本: v0.1
> **⚠️ 重建说明**: 本文档因 H4 修复脚本误操作 + git restore 删除后重建, 内容精简, 完整接口见 [`../规范/16_接口契约汇总.md`](../规范/16_接口契约汇总.md)。

## 借鉴源

[mempal](E:\giti\mempal) 5 状态 Knowledge lifecycle + Promote Gate。

## 问题

千寻 memory 现在**无 status 字段**, 写入的"知识"全部平级, 没法区分:
- 用户随口说的"我喜欢 X" (噪音)
- 经过 5+ 次 distilled 的"X = Y" (真知识)

需要**5 状态 lifecycle** + **Gate 评估 promote**。

## 方案

### 14.1 5 状态 enum

```rust
// qianxun-memory/src/knowledge.rs
pub enum KnowledgeStatus {
    Draft,       // 刚 flush 进来的原始数据
    Candidate,   // 通过初筛 (非空 + 主题相关)
    Promoted,    // 验证通过 (3+ 引用 + 无 counterexample)
    Canonical,   // 多源验证 + 长期稳定
    Archived,    // 过期/冲突, 不再使用
}
```

### 14.2 Promote Request + Outcome

```rust
pub struct PromoteRequest {
    pub knowledge_id: String,
    pub target_status: KnowledgeStatus,
    pub verification_refs: Vec<String>,
    pub reason: String,
    pub reviewer: Option<String>,
    pub allow_counterexamples: bool,
    pub enforce_gate: bool,
}

pub fn promote_knowledge(db: &Database, req: PromoteRequest) -> Result<PromoteOutcome>;
```

### 14.3 Gate 规则

```rust
// qianxun-memory/src/gate.rs
pub struct KnowledgeGate;

impl KnowledgeGate {
    pub fn evaluate_promotion(knowledge: &Knowledge, refs: &[Reference]) -> GateVerdict;
    // Gate 规则: Draft → Candidate 需 5+ 引用, Candidate → Promoted 需无 counterexample + 3 引用
}
```

## 跟其他缺口的关系

- 跟 [缺口 06](./06_压缩前MemoryFlush.md) 联动: flush 调 `memory.save()` (status=Draft), 异步触发 `evaluate_promotion`
- 跟 [缺口 11](./11_Verdict四态与BDD验收.md) 联动: Verdict 验证通过可加速 promote

## 文件改动

- `qianxun-memory/src/knowledge.rs` (新) 5 状态 + PromoteRequest/Outcome
- `qianxun-memory/src/gate.rs` (新) KnowledgeGate
- `qianxun-core/src/hooks/builtin/reflect.rs` 联动 promote

**总计 ~420 行** (含 5 状态 transition + Gate 拒绝测试)

## 不做什么

- 不做 ML-based 分类 (用引用次数 + 规则)
- 不做跨 session 共享 (单 session 知识库足够)
- 不做 expire 自动 archive (用户显式操作)

## 验收

- [ ] 5 状态 transition 测试
- [ ] Gate 拒绝测试 (counterexample)
- [ ] `cargo test -p qianxun-memory -- knowledge` 全过
