# 缺口 13: Knowledge 五状态与 Gate

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[mempal](E:\git\ai\mempal) `knowledge_lifecycle.rs`: 5 状态 + Promote/Demote + KnowledgeGate (有 verification_refs 才允许 promote)。

## 问题

千寻 `qianxun-memory` 是简单 SQLite key-value, **无 status 状态机**:

- LLM 写 memory 的信息无法标记"已验证"
- 用户偏好和错误信息混在一起
- 没人 review, 长期下来 memory 噪声大

## 方案

### 13.1 5 状态 lifecycle

```rust
// qianxun-memory/src/knowledge.rs (新)

pub enum KnowledgeStatus {
    Draft,       // LLM 刚写下, 未验证
    Candidate,   // 满足 promote 候选条件
    Promoted,    // 通过 gate, 标记为可靠
    Canonical,   // 长期真理 (用户明确确认)
    Archived,    // 30+ 天未访问 + importance < 0.3
}
```

### 13.2 PromoteRequest + Gate

```rust
pub struct PromoteRequest {
    pub knowledge_id: String,
    pub target_status: KnowledgeStatus,   // Candidate | Promoted | Canonical
    pub verification_refs: Vec<String>,   // 必须是 evidence (e.g. 文档链接, 引用)
    pub reason: String,                   // 为什么 promote
    pub reviewer: Option<String>,         // 人工 reviewer (可省略, 系统自动)
    pub allow_counterexamples: bool,      // 是否有反例
    pub enforce_gate: bool,               // 是否强制过 gate
}

pub fn promote_knowledge(db: &Database, req: PromoteRequest) -> Result<PromoteOutcome> {
    // 1. 验证 target_status 是 Promoted / Canonical
    // 2. 验证 verification_refs 至少 1 个
    // 3. evaluate_gate_for_drawer (知识 gate):
    //    - 必须有 reviewer (人工) 或 importance ≥ 0.8
    //    - 反例 ≤ 2
    //    - 引用 ≥ 1
    // 4. gate 通过 → 改 status
    // 5. gate 失败 → 报错, status 不变
}
```

### 13.3 KnowledgeGate

```rust
pub struct GateReport {
    pub allowed: bool,
    pub reasons: Vec<String>,  // 失败原因
}

pub fn evaluate_gate_for_drawer(
    db: &Database,
    drawer: &KnowledgeItem,
    target: &KnowledgeStatus,
    reviewer: Option<&str>,
    allow_counterexamples: bool,
) -> Result<GateReport>;
```

### 13.4 ReflectHook 联动

```rust
// qianxun-core/src/hooks/builtin/reflect.rs (改)

async fn handle(&self, ctx: HookContext<'_>) -> HookResult {
    // reflect 通过 → 触发 promote Candidate → Promoted
    if let Some(reflection_result) = self.evaluate(&ctx).await {
        if reflection_result.confidence >= 0.8 {
            for item_id in reflection_result.knowledge_ids {
                let _ = promote_knowledge(db, PromoteRequest {
                    knowledge_id: item_id,
                    target_status: KnowledgeStatus::Promoted,
                    verification_refs: vec!["reflect-pass".into()],
                    reason: format!("reflect confidence {}", reflection_result.confidence),
                    reviewer: None,
                    allow_counterexamples: true,
                    enforce_gate: true,
                });
            }
        }
    }
    HookResult::Continue
}
```

### 13.5 5 状态转换图

```text
[LLM 写] → Draft
    │
    │ (使用 ≥ 3 次 + importance ≥ 0.6)
    ↓
Candidate
    │
    │ (gate: refs ≥ 1, reviewer 确认, no counter)
    ↓
Promoted ──── (用户显式确认 + 5+ 次使用)
    │         ↓
    │     Canonical
    │
    │ (30+ 天未访问 + importance < 0.3)
    ↓
Archived
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-memory/src/knowledge.rs` (新) | 5 状态 + promote/demote | +200 |
| `qianxun-memory/src/gate.rs` (新) | KnowledgeGate | +80 |
| `qianxun-core/src/hooks/builtin/reflect.rs` | 联动 promote | +30 |
| `qianxun-memory/src/persistence.rs` | + 字段 (status, refs, importance) | +30 |
| 测试 | 5 状态 + gate 拒绝 | +80 |

**总计 ~420 行**

## 不做什么

- 不做 knowledge 自动 review (reviewer 人工显式确认)
- 不做 knowledge 跨 session 共享
- 不做 importance 自动计算 (LLM 写时显式给)

## 验收

- [ ] LLM 写 memory → status=Draft
- [ ] Draft 3 次使用 + importance 0.7 → auto → status=Candidate
- [ ] Candidate 调 promote (无 refs) → gate 失败, status 不变
- [ ] Candidate 调 promote (1 ref) → gate 通过, status=Promoted
- [ ] 用户显式 confirm → status=Canonical
- [ ] 30 天未访问 → auto → status=Archived
