# Codex 上下文管理分析

> 状态: 完成 | 2026-05-28
> 基于 codex-rs (Bazel + Rust workspace, 80+ crates)

---

## 核心结论

Codex **不使用滑动窗口或暴力截断**。它采用一种称为"压缩"(Compaction) 的策略——用 LLM 对旧轮次做摘要，保留最近消息，并有一整套机制确保 tool_use/tool_result 配对完整性。

---

## 一、总体架构

```
Session (会话顶层)
├── SessionState
│   ├── ContextManager     ← 内存中的完整消息历史
│   └── AutoCompactWindow  ← Token 增长追踪
├── RegularTask            ← 标准轮次循环
├── CompactTask            ← 手动 /compact 触发
└── SessionService         ← 持久化、回滚
```

### ContextManager

```rust
struct ContextManager {
    items: Vec<ResponseItem>,       // 所有消息，从旧到新
    history_version: u64,           // 每次重写递增
    token_info: Option<TokenUsageInfo>,  // 基于 API 响应的精确 token 计数
    reference_context_item: Option<TurnContextItem>,  // 基线
}
```

关键方法 `for_prompt()`：在发送给模型前对历史的**克隆**做规范化（normalize），不修改原始历史。

---

## 二、三种压缩策略

### 2.1 本地压缩 v1（Memento 策略）

文件：`core/src/compact.rs`

流程：

```
1. 检测到 token 超限
2. 调用 LLM 自身，使用专用提示词模板生成摘要
   → 模板: templates/compact/prompt.md
3. 构建压缩后的历史：
   a. 所有用户消息（每条截断至 20K tokens）
   b. 摘要文本（带 SUMMARY_PREFIX 标记）
   c. 如果是轮次中压缩，在最后一条用户消息前重新注入初始上下文
4. replace_compacted_history() 写入新历史
```

### 2.2 远程压缩 v1

文件：`core/src/compact_remote.rs`

通过 `responses/compact` 专用 API 端点。发送前调用 `trim_function_call_history_to_fit_context_window()` 从末尾删除工具调用/输出，直到适合模型窗口。

### 2.3 远程压缩 v2

文件：`core/src/compact_remote_v2.rs`

通过标准 `/responses` 流式端点，附加 `CompactionTrigger` 项。API 返回 `ResponseItem::Compaction`，服务端负责实际压缩逻辑。

---

## 三、触发时机

### 3.1 预采样压缩（轮次前）

在 `run_turn()` 顶部，向 LLM 发送第一条消息之前：

```rust
run_pre_sampling_compact()  // 检查 token 是否超限
```

### 3.2 轮次中压缩

在每个采样请求之后：

```rust
if token_limit_reached && needs_follow_up {
    run_auto_compact(..., BeforeLastUserMessage, ..., MidTurn)
}
```

条件：
- `token_limit_reached`：token 已达到限制
- `needs_follow_up`：模型仍有待完成的工作（待处理工具调用或待消费输入）

### 3.3 手动压缩

用户通过 `/compact` 命令触发 → `CompactTask`。

### 3.4 回退截断

极罕见情况：压缩过程本身也遇到 `ContextWindowExceeded`，则调用 `remove_first_item()` 删除最旧项（成对删除）。

---

## 四、工具调用配对完整性 — normalize.rs

这是最精密的部分。`for_prompt()` 在发送前对历史做规范化：

### 4.1 ensure_call_outputs_present()

扫描所有 `FunctionCall` / `ToolSearchCall` / `CustomToolCall` / `LocalShellCall`，检查是否有对应的输出。**没有则合成 "aborted" 文本的 `FunctionCallOutput`** 插入其后。

```
[asst: FunctionCall(A)]              → [asst: FunctionCall(A)]
[user: FunctionCallOutput(A)]        → [user: FunctionCallOutput(A)]
[asst: FunctionCall(B)]              → [asst: FunctionCall(B)]
     ↓ 没有 FunctionCallOutput(B)        ↓ 自动补入
                                     → [user: FunctionCallOutput(B, "aborted")]
```

### 4.2 remove_orphan_outputs()

反向操作：扫描输出项，删掉没有对应调用的孤立结果。

### 4.3 remove_corresponding_for()

删除项时自动删除其配对项，保证成对删除。

### 四种配对类型

| 调用类型 | 输出类型 |
|----------|----------|
| `FunctionCall` | `FunctionCallOutput` |
| `ToolSearchCall` | `ToolSearchOutput` |
| `CustomToolCall` | `CustomToolCallOutput` |
| `LocalShellCall` | `FunctionCallOutput` |

---

## 五、Token 追踪 — AutoCompactWindow

文件：`core/src/state/auto_compact_window.rs`

```rust
struct AutoCompactWindow {
    ordinal: u64,                                // 窗口计数器（每次压缩递增）
    prefill_input_tokens: Option<Prefill>,        // 基线：ServerObserved(i64) | Estimated(i64)
}
```

### 两种作用域

配置项 `model_auto_compact_token_limit_scope`：

- **`Total`** — 总上下文 token 达到限制时触发
- **`BodyAfterPrefix`（默认）** — 仅计算"超出预填充基线"的增长部分

BodyAfterPrefix 的计算：
```
增长量 = 总 token - 预填充基线（系统提示词 + 工具定义）
```

预填充优先用 API 返回的 ServerObserved 值，没有则用本地估算。

---

## 六、与千寻的对比

| 维度 | Codex | 千寻当前 |
|------|-------|---------|
| token 追踪 | API 精确值 | 无追踪 |
| 阈值策略 | 可配置 Total/BodyAfterPrefix | 无 |
| 历史压缩 | LLM 摘要 + 保留最近用户消息 | 无 |
| tool_use/tool_result 配对 | normalize.rs 自动修复 | 无保护 |
| tool result 截断 | 压缩时自动处理 | 无 |
| 配置项 | `model_auto_compact_token_limit` | 无 |

---

## 七、Claude Code 官方方案（补充）

> 来源：《Claude Code 核心系统篇》第 7 章 — 上下文管理

### 7.1 四级渐进压缩

与 Codex 的单级压缩不同，Claude Code 官方采用四级渐进式策略，代价逐级递增：

| 级别 | 名称 | 代价 | 触发条件 | 做法 |
|------|------|------|----------|------|
| L1 | **Snip** | 零 LLM | 任意时刻 | 旧 tool result → `[已清除]` 标记 |
| L2 | **MicroCompact** | 零 LLM | 距上次 assistant 超过缓存 TTL | 保留最近 N 个 tool result，其余清除 |
| L3 | **Collapse** | 少量 LLM | 使用率达 90% | 选择性重编消息组；95% 时阻塞新轮次 |
| L4 | **AutoCompact** | 全 LLM | 超过阈值 | 生成完整摘要替换旧轮次 |

**Snip 是最巧妙的设计**：每次 read 10 个大文件后，分析完立刻把 tool result 替换为简短标记，零成本释放 ~15K tokens。保留消息结构（不删消息本身），只清空内容。

### 7.2 有效窗口公式

```
有效窗口 = 模型窗口 - min(max_output_tokens, 20_000)
```

预留 20K tokens 给压缩时 LLM 生成摘要使用，防止压缩本身因空间不足失败。

### 7.3 四区安全网

| 区间 | 水位 | 行为 |
|------|------|------|
| Safe | 0%–85% | 正常运行 |
| Warning | 85%–90% | 黄色预警提示用户 |
| Danger | 90%–95% | 触发 AutoCompact |
| Blocked | 95%–100% | 拒绝新请求 |

### 7.4 熔断器

连续 3 次 AutoCompact 失败后熔断，不再重试。基于 1,279 个会话的实际数据——熔断前每天浪费 ~25 万次 API 调用。

### 7.5 摘要提示词结构

双段输出：

```
<analysis>
  思考过程：哪些信息重要、如何组织
  此块在最终结果中被丢弃
</analysis>
<summary>
  9 个结构化章节的正式摘要
  此块进入上下文窗口
</summary>
```

### 7.6 CompactBoundaryMessage

压缩后在消息链中插入边界标记，携带压缩前 token 数和消息数。后续操作通过 `logicalParentUuid` 识别"哪些消息已被压缩过"，避免重复压缩。

### 7.7 重注入预算控制

压缩后附件重注入严格限制：

| 限制 | 值 |
|------|-----|
| 总计预算 | 50K tokens |
| 单文件上限 | 5K tokens |
| 文件数量上限 | 5 个 |
| 单技能上限 | 5K tokens |
| 技能总预算 | 25K tokens |

防止"压缩-膨胀-再压缩"的震荡循环。

---

## 八、对千寻的路线更新

结合 Codex 源码分析和 Claude Code 官方设计，修正后的四阶段路线：

### Phase 1 — 源头截断 + Snip（零 LLM 代价）
- `execute_command` 输出 >8K 截尾
- 引入 Snip：旧 tool result 替换为简短标记
- 效果：同时控制增量膨胀和存量回收

### Phase 2 — Normalize 配对保护（安全地基）
- 发送前自动修复 tool_use/tool_result 配对
- 一切历史操作的前提

### Phase 3 — Token 水位 + 熔断器
- API 精确值追踪
- 四区安全网 + BodyAfterPrefix 基线
- 连续 3 次压缩失败熔断

### Phase 4 — AutoCompact
- 超限时 LLM 摘要压缩
- 双段提示词（analysis/summary）
- CompactBoundaryMessage 标记
- 50K 重注入预算

### 关键原则

1. 所有历史操作必须先有 Normalize
2. 零 LLM 代价的操作优先（Snip > MicroCompact > Collapse > AutoCompact）
3. 有效窗口 = 模型窗口 - 预留输出 token
4. 压缩后严格限制重注入，避免震荡
