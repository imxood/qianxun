# 缺口 09: Hook 五层 Tier

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core / qianxun-runtime | 最后更新: 2026-06-10 | 版本: v0.1

## 借鉴源

[oh-my-opencode](E:\git\ai\oh-my-opencode) 62 lifecycle hooks 分 5 tier: Session(24) / ToolGuard(17) / Transform(5) / Continuation(7) / Skill(2)。

## 问题

千寻 v2 HookRegistry 是 4 槽位 (BeforeTool/AfterTool/BeforePrompt/LoopEnd), hook 多了会**乱**: 25 个 hook 都堆 BeforeTool, 顺序难管, 调试难。

## 方案

### 9.1 HookTier 枚举

```rust
// qianxun-core/src/hooks/tier.rs (新)

pub enum HookTier {
    /// 整个 session 生命周期 (CreateSession / Pause / Resume / Delete)
    Session,
    /// 工具调用保护 (Permission / Filter / Audit)
    ToolGuard,
    /// 消息转换 (Sanitize / Compress / Inject)
    Transform,
    /// 后台续接 (BackgroundTask / Resume / Handoff)
    Continuation,
    /// 技能注入 (LoadSkill / PlanInject)
    Skill,
}
```

### 9.2 HookChain per tier

```rust
pub struct HookRegistry {
    chains: HashMap<HookTier, Vec<Arc<dyn HookHandler>>>,
    stats: HookStats,  // 缺口 01
}

impl HookRegistry {
    pub fn register(&mut self, tier: HookTier, handler: Arc<dyn HookHandler>) {
        self.chains.entry(tier).or_default().push(handler);
    }

    /// 5 tier × 4 时机 = 20 触发位置
    pub async fn dispatch(&self, tier: HookTier, event: HookEvent<'_>) -> HookResult {
        let chain = self.chains.get(&tier).cloned().unwrap_or_default();
        for handler in chain {
            // 顺序触发, 任一 Block 立即返回
        }
    }
}
```

### 9.3 触发位置矩阵

|         | Session | ToolGuard | Transform | Continuation | Skill |
|---------|---------|-----------|-----------|--------------|-------|
| BeforeToolCall |          | ✅        |           |              |       |
| AfterToolCall  |          | ✅        |           |              |       |
| BeforePromptBuild |      |           | ✅        |              | ✅    |
| LoopEnd        | ✅       |           |           | ✅            |       |

### 9.4 内置 hook 重新归类

| 原 builtin/ | 归类到 tier |
|---|---|
| `plan_gate.rs` | Session (loop 入口) |
| `permission.rs` | ToolGuard |
| `plan.rs` (自主) | Skill (注入 plan step) |
| `reflect.rs` | Transform (改写消息) |
| `workflow.rs` | Session (阶段切换) |
| `subagent.rs` | Continuation (fork) |

## 文件改动

| 文件 | 改动 | 行数 |
|---|---|---|
| `qianxun-core/src/hooks/tier.rs` (新) | HookTier | +50 |
| `qianxun-core/src/hooks/registry.rs` | 改 HashMap<HookTier, Vec<...>> | +30 |
| builtin/ 6 文件 | 加 tier 标注 | +30 |
| 测试 | 5 tier 独立调度 | +40 |

**总计 ~150 行**

## 不做什么

- 不做 hook 间通信 (一个 hook 写 ctx 给下一个) — 简单为先
- 不做 hook 动态 enable/disable (用 config 静态)
- 不做 hook 优先级 (同 tier 顺序触发)

## 验收

- [ ] 5 tier 独立注册
- [ ] Session tier 在 create_session 触发
- [ ] ToolGuard tier 只在 BeforeToolCall / AfterToolCall 触发
- [ ] 25 hook 平均分布 5 tier, 每个 tier 5 个
- [ ] 单 tier 失败不影响其他 tier
