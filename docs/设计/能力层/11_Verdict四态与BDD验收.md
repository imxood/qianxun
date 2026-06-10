# 缺口 11: Verdict 四态与 BDD 验收

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[agent-spec](E:\git\ai\agent-spec) Verdict 四态 `pass/fail/skip/uncertain` + BDD DSL。

## 问题

千寻验收完全靠**人工 6 步 E2E**, 无 in-loop 自验:

- LLM 跑完说"完成", 实际没完成 (缺测试 / 编译失败)
- 反射 hook 知道有问题, 但没法**判定"做完了"**
- 用户得自己跑 `cargo test`, 看输出

## 方案

### 11.1 Verdict 4 态

```rust
// qianxun-core/src/verify/mod.rs (新)

pub enum Verdict {
    Pass,       // 完全通过
    Fail,       // 明确失败, 重试/转人工
    Skip,       // 跳过 (≠ Pass, 阻塞流水线)
    Uncertain,  // 不确定, 需人工
}

pub struct VerifyResult {
    pub verdict: Verdict,
    pub evidence: Vec<VerifyEvidence>,  // 实际跑的命令 + 输出
    pub confidence: f32,
    pub reason: String,
}

pub struct VerifyEvidence {
    pub command: String,    // "cargo test"
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
```

### 11.2 BDD Spec 格式

```rust
pub struct BddSpec {
    pub name: String,
    pub intent: String,                    // "确保 hashline edit 拒绝 stale"
    pub given: Vec<SpecStep>,              // 前提
    pub when: Vec<SpecStep>,               // 动作
    pub then: Vec<SpecStep>,               // 期望
    pub boundary: Vec<PathGlob>,           // 涉及的文件 glob
    pub test_selector: String,             // "cargo test hashline::"
}

pub struct SpecStep {
    pub action: String,                    // "read file X"
    pub expect: String,                    // "return hashlined content"
}
```

### 11.3 Verifier 工具

LLM 调用 `verify_spec(spec) -> VerifyResult`:

```rust
pub async fn verify_spec(spec: BddSpec) -> VerifyResult {
    // 1. 机械执行 test_selector (如 "cargo test hashline::")
    // 2. 解析 stdout/stderr → 计数 passed / failed
    // 3. 全 passed → Pass
    //    全 failed → Fail
    //    部分 failed → Uncertain (with reason)
    //    不可执行 → Skip (e.g. cargo not found)
    // 4. 检查 boundary glob: 涉及的文件都被更新了
}
```

### 11.4 ReflectHook 接入

```rust
// qianxun-core/src/hooks/builtin/reflect.rs (改)

async fn handle(&self, ctx: HookContext<'_>) -> HookResult {
    // 1. 跑 verify_spec (if spec exists)
    let result = if let Some(spec) = &self.bdd_spec {
        verify_spec(spec.clone()).await
    } else {
        Verdict::Skip  // 无 spec, 不强验
    };

    // 2. 决策
    match result.verdict {
        Verdict::Pass => HookResult::Continue,
        Verdict::Skip => HookResult::Continue,
        Verdict::Fail => HookResult::Modify(/* retry args */),
        Verdict::Uncertain => HookResult::Block("需人工确认".into()),
    }
}
```

### 11.5 SseEvent

```rust
// qianxun-runtime/src/sse.rs

VerifyStarted { spec_name, test_selector },
VerifyCompleted { spec_name, verdict, evidence_summary },
```

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/verify/mod.rs` (新) | Verdict + Verifier | +180 |
| `qianxun-core/src/verify/bdd.rs` (新) | BddSpec | +50 |
| `qianxun-core/src/hooks/builtin/reflect.rs` | 接入 verifier | +40 |
| `qianxun-runtime/src/sse.rs` | +2 变体 | +15 |
| 测试 | 4 态各 1 case | +60 |

**总计 ~345 行**

## 不做什么

- 不做 BDD DSL 完整实现 (只取 Spec 简化版)
- 不做 Verify 调度系统 (LLM 显式调 verify_spec 工具)
- 不做 verify 结果持久化 (单 session 周期内)

## 验收

- [ ] spec 写明 "hashline edit 拒绝 stale" → 跑 cargo test → Verdict::Pass
- [ ] spec 写明 "sub-agent 工具白名单" → 跑白名单测试 → Verdict::Pass
- [ ] cargo 失败 → Verdict::Fail + evidence 显示 panic
- [ ] cargo 不存在 → Verdict::Skip
- [ ] 部分测试过 → Verdict::Uncertain
