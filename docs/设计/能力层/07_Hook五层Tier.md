# 缺口 07: Hook 五层 Tier

> 状态: 草稿 (待 code review) | 适用范围: qianxun-core | 最后更新: 2026-06-11 | 版本: v0.1
> **⚠️ 重建说明**: 本文档因 H4 修复脚本误操作 + git restore 删除后重建, 内容精简, 完整接口见 [`../规范/16_接口契约汇总.md`](../规范/16_接口契约汇总.md)。

## 借鉴源

[oh-my-opencode](E:\giti\oh-my-opencode) 5 层 Hook 调度: Session / ToolGuard / Transform / Continuation / Skill。

## 问题

千寻 v2 HookRegistry 是 `HashMap<HookEvent, Vec<Arc<dyn HookHandler>>>`, 所有 hook 堆一槽位, 25+ hook 后调度顺序难管, 调试不友好。

需要 5 层 tier 把不同关注点**解耦**。

## 方案

### 7.1 HookTier 5 变体

```rust
// qianxun-core/src/hooks/tier.rs
pub enum HookTier {
    Session,       // 整个 session 生命周期 (开/关/暂停)
    ToolGuard,     // 工具调用前后 (权限/审计/限流)
    Transform,     // 上下文/请求变换 (变量替换/脱敏/压缩)
    Continuation,  // 流程控制 (loop/sub-agent/reflection)
    Skill,         // skill 触发 (自动加载/蒸馏)
}
```

### 7.2 HookChain per tier

```rust
pub struct HookRegistry {
    chains: HashMap<HookTier, Vec<Arc<dyn HookHandler>>>,
    stats: DashMap<String, Arc<HookStats>>>,  // 跟 [缺口 01](./01_Hook退出码与熔断.md) 共享
}
```

### 7.3 触发位置矩阵

| HookEvent | Session | ToolGuard | Transform | Continuation | Skill |
|---|---|---|---|---|---|
| BeforePromptBuild | | | ✓ | | ✓ |
| AfterPromptBuild | | | ✓ | | |
| BeforeToolCall | | ✓ | ✓ | | |
| AfterToolCall | | ✓ | | ✓ | |
| BeforeLoopIter | | | | ✓ | |
| AfterLoopIter | | | | ✓ | ✓ |

## 跟其他缺口的关系

- 跟 [缺口 01](./01_Hook退出码与熔断.md) 共享 `HookStats` + circuit breaker
- 5 tier 跟 builtin/ 6 个 handler 一一标注 (各 handler 选 1 个 tier)

## 文件改动

- `qianxun-core/src/hooks/tier.rs` (新) HookTier 5 变体
- `qianxun-core/src/hooks/registry.rs` 改 `HashMap<HookTier, Vec<...>>`
- `qianxun-core/src/hooks/builtin/*.rs` 6 文件加 tier 标注

**总计 ~150 行** (含 5 tier 独立调度 + 6 builtin 标注测试)

## 不做什么

- 不做 hook 优先级 (按注册顺序即可, 显式优先级留 v3+)
- 不做动态 tier 加载 (tier 列表写死)

## 验收

- [ ] 5 tier 独立调度测试
- [ ] builtin 6 handler 各归 1 个 tier 测试
- [ ] `cargo test -p qianxun-core -- hooks::tier` 全过
