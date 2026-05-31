# 千寻 Agent 模式设计

> 版本: 0.2 | 更新: 2026-05-31 | 状态: 草案
>
> Agent 模式是 AgentLoop 的上层状态机——决定如何调用 LLM、何时执行工具、何时结束

---

## 1. 设计目标

### 核心理念

> **Agent 模式不是提示词模板，而是 AgentLoop 的控制逻辑。**

当前千寻只有一个固定的 React 循环。用户面对不同复杂度任务时，模式应该自适应：

| 模式 | 适合的任务 | 决策方式 |
|---|---|---|
| **React** | 简单问答、单步工具调用 | 每次 LLM 调用即时决定下一步 |
| **Plan-and-Execute** | 多文件重构、新功能实现 | Agent 先制定计划，再逐条执行 |
| **Reflective** | 排障、测试修复、质量敏感任务 | 执行后自检一轮再输出 |
| **Workflow** | 可重复过程（code review、发布、排障） | 按用户/模板预设的阶段序列执行 |

### 非目标

- 多 Agent 编排 / Delegation（千寻是单 Agent 系统）
- 动态切换模式（模式和任务绑定，一个 session 内不切换）

---

## 2. 架构

### 2.1 在 AgentLoop 中的位置

```
handle_user_message()
  │
  ├─ system_prompt.rs     ← 根据 pattern 注入不同指令
  ├─ pattern.rs           ← 选择并执行对应的循环（新增）
  │   ├─ React:           engine.rs 的现有循环
  │   ├─ PlanAndExecute:  plan.rs 的两阶段循环
  │   ├─ Reflective:      reflect.rs 的双轮循环
  │   └─ Workflow:        workflow.rs 的阶段序列循环
  │
  └─ 返回一致的结果（对 OutputSink 透明）
```

### 2.2 文件结构

```
qianxun-core/src/agent/
├── mod.rs              # AgentPattern enum, 公开 API
├── message.rs          # Message 类型（已有）
├── conversation.rs     # 会话管理（已有）
├── engine.rs           # React 循环（已有 processing_loop）
├── system_prompt.rs    # 提示词组装（已有，新增 pattern 注入）
├── pattern.rs          # 新增：模式选择器 + 状态机
├── plan.rs             # 新增：Plan-and-Execute 两阶段循环
├── reflect.rs          # 新增：Reflective 双轮循环
└── workflow.rs         # 新增：Workflow 阶段序列循环
```

### 2.3 AgentPattern 枚举

```rust
// === agent/pattern.rs

/// Agent 工作模式
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPattern {
    /// React: 思考 → 行动 → 观察 循环（默认）
    React,
    /// Plan-and-Execute: 先制定计划再逐步执行
    PlanAndExecute,
    /// Reflective: 执行后自检一轮
    Reflective,
    /// Workflow: 按预设阶段序列执行
    Workflow {
        /// 使用的模板名称
        template: WorkflowTemplateId,
    },
}

impl Default for AgentPattern {
    fn default() -> Self { Self::React }
}
```

### 2.4 入口调度

```rust
// === agent/mod.rs

impl AgentLoop {
    pub async fn handle_user_message(
        &self,
        conversation: &mut Conversation,
        user_input: &str,
        sink: &dyn OutputSink,
        workspace: Option<&Workspace>,
    ) -> Result<AgentResult> {
        match &self.config.pattern {
            AgentPattern::React => {
                self.handle_react(conversation, user_input, sink, workspace).await
            }
            AgentPattern::PlanAndExecute => {
                self.handle_plan_and_execute(conversation, user_input, sink, workspace).await
            }
            AgentPattern::Reflective => {
                self.handle_reflective(conversation, user_input, sink, workspace).await
            }
            AgentPattern::Workflow { template } => {
                self.handle_workflow(conversation, user_input, sink, workspace, template).await
            }
        }
    }
}
```

---

## 3. 工具权限门控

### 3.1 核心设计

从 Claude Code 的 Plan Mode 学到的关键设计：**模式通过工具权限控制行为，不依赖提示词约束**。

```rust
impl AgentPattern {
    pub fn allowed_tool_categories(&self) -> ToolCategoryFilter {
        match self {
            AgentPattern::PlanAndExecute if in_plan_phase() => ToolCategoryFilter::read_only(),
            // Workflow 各个阶段有自己的工具过滤器（由模板定义）
            AgentPattern::Workflow { template } => template.current_stage_tool_filter(),
            _ => ToolCategoryFilter::all(),
        }
    }
}
```

### 3.2 工具分类

`ToolCategory` 枚举定义在 `qianxun-core/src/tools/mod.rs` 中，**Agent Patterns 依赖此枚举**。实现 Phase 3b 前必须先完成此定义的落地。

```rust
// === qianxun-core/src/tools/mod.rs 新增

#[derive(Debug, Clone, Copy)]
pub enum ToolCategory {
    Read,       // read_file, list_directory
    Write,      // write_file, edit_file
    Search,     // grep, search
    Terminal,   // terminal 命令执行
    Network,    // MCP HTTP 工具等
    Think,      // 无副作用的思考工具
}
```

### 3.3 门控的不只是工具——系统提示词也配合

```rust
// system_prompt.rs 新增

pub fn build_mode_instructions(pattern: &AgentPattern) -> String {
    match pattern {
        AgentPattern::React => String::new(),
        AgentPattern::PlanAndExecute => {
            "\n## 当前模式：Plan-and-Execute\n\
             第一阶段（Plan）：分析需求，制定执行计划。此阶段只能读文件、搜索，不能修改。\n\
             第二阶段（Execute）：按计划逐步执行。\n\
             你的输出应以 `## Plan` 或 `## Execute` 标记当前阶段。".into()
        }
        AgentPattern::Reflective => {
            "\n## 当前模式：Reflective\n\
             完成修改后，请检查自己的 diff 是否有遗漏或错误。\n\
             如果有问题，继续修正；否则输出最终结果。".into()
        }
        AgentPattern::Workflow { template } => {
            format!(
                "\n## 当前模式：Workflow —— {}\n{}\n\n\
                 当前阶段是：**{}**\n{}\n\
                 请严格按照当前阶段的要求执行，不要跨越阶段。\n\
                 完成当前阶段后输出 `## Stage Complete: {}` 进入下一阶段。",
                template.name,
                template.description,
                template.current_stage().name,
                template.current_stage().instructions,
                template.current_stage().name,
            )
        }
    }
}
```

---

## 4. React 模式（默认）

```
handle_react()
  ├─ enforce_budget()
  ├─ build_request() → 注入 system_prompt + memory + skills + pattern 指令
  └─ processing_loop::handle_user_message()
       → provider.stream_completion()
       → 对每个 event:
          Text     → sink.on_text()
          Thinking → sink.on_thinking()
          ToolCall → tools.execute_async() → push results → 继续
          Stop     → 有 tool_use 则继续，否则结束
```

复用现有 engine.rs，无需改动。

---

## 5. Plan-and-Execute 模式

两阶段循环：**计划阶段（只读）→ 用户确认 → 执行阶段（全部工具）**。

**用户确认超时**：发送 `session/plan` 通知后，等待用户决策最多 **5 分钟**。
超时未响应 → 自动取消（`Cancelled`），关闭 session。
CLI 模式下超时由用户主动选择（阻塞等待），无超时限制。

具体设计见 §9（状态机）。复用 `plan.rs`。

---

## 6. Reflective 模式

双轮循环：**React 阶段 → 自检 → 修正（如果需要）**。

具体设计见 §9（状态机）。复用 `reflect.rs`。

---

## 7. Workflow 模式（新增）

### 7.1 核心概念

```
Workflow = 预设阶段的线性序列
每个阶段 = { name, description, instructions, allowed_tools }
阶段之间是顺序的，Agent 完成一个阶段后才进入下一个
```

```
Stage 1: analyze          Stage 2: fix              Stage 3: verify
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│ 工具: read_only  │ ──→ │ 工具: write      │ ──→ │ 工具: terminal   │
│ 指令: 阅读代码   │     │ 指令: 实施修复   │     │ 指令: 编译测试   │
│ 输出: 分析报告   │     │ 输出: 修改内容   │     │ 输出: 验证结果   │
└─────────────────┘     └─────────────────┘     └─────────────────┘
```

### 7.2 WorkflowTemplate 数据结构

```rust
// === agent/workflow.rs

/// 工作流模板 ID
pub type WorkflowTemplateId = String;

/// 工作流模板
pub struct WorkflowTemplate {
    pub id: WorkflowTemplateId,      // "code-review"
    pub name: String,                 // "代码审查"
    pub description: String,          // "对当前变更进行代码审查"
    pub stages: Vec<WorkflowStage>,   // 阶段序列（线性）
}

/// 工作流中的一个阶段
pub struct WorkflowStage {
    pub name: String,                 // "analyze"
    pub description: String,          // "分析变更内容"
    pub instructions: String,         // 注入 system prompt 的指令
    pub allowed_tools: ToolCategoryFilter,  // 此阶段可用的工具
    pub exit_marker: &'static str,    // 阶段结束标记，Agent 输出此标记时进入下一阶段
}
```

### 7.3 Workflow 循环

```
handle_workflow()
  │
  ├─ 加载模板
  │   └─ match template.id → 内置模板 / 用户自定义模板
  │
  ├─ 处理各阶段（线性序列）
  │   for each stage in template.stages:
  │     │
  │     ├─ 注入当前阶段的 system prompt + 工具过滤器
  │     │
  │     ├─ sink.on_stage_transition(stage.name)
  │     │   → CLI: 打印 "🔷 阶段：{stage.name} — {stage.description}"
  │     │   → ACP: 发送 session/stage 通知
  │     │
  │     ├─ 子循环：React 模式（复用 engine.rs）
  │     │   → 限制 max_turns = stage_max_turns
  │     │   → 通过 execute_with_filter() 执行工具
  │     │   → 持续到：Agent 输出 exit_marker 或达到 max_turns
  │     │
  │     └─ 收集阶段输出 → 注入下一阶段的上下文
  │
  └─ 汇总 → sink.on_text(summary)
```

### 7.4 Agent 视角的 Workflow 体验

用户输入 `/workflow code-review` 后，Agent 看到的是：

```
对话开始（system prompt 包含）：

## 当前模式：Workflow —— 代码审查
对当前变更进行逐文件代码审查

### 当前阶段：analyze
阅读所有变更文件，理解改动的范围和目的。
列出每个文件的变更要点。

可用工具：read_file, grep, search, list_directory, think
请勿在此阶段修改文件。

完成分析后，输出 `## Stage Complete: analyze` 进入下一阶段。
```

Agent 完成 analyze 后，`exit_marker` 匹配 → 进入下一阶段 fix，system prompt 更新为：

```
### 当前阶段：fix
对每个变更文件进行逐行审查。检查：
- 逻辑正确性
- 边界情况处理
- 代码风格一致性

记录发现的问题和改进建议。

可用工具：read_file, grep, think
请勿在此阶段修改文件。

完成审查后，输出 `## Stage Complete: fix` 进入下一阶段。
```

### 7.5 内置模板

千寻预装以下 Workflow 模板：

#### 模板 1: code-review

```
名称: 代码审查
适用: /workflow code-review <path>

Stage 1: analyze（只读）
  → 读取 diff/文件，理解变更范围和目的
Stage 2: review（只读）
  → 逐行审查每个文件，列出问题和建议
Stage 3: summarize（只读）
  → 汇总发现，给出总体评估和修改建议
```

#### 模板 2: bug-fix

```
名称: Bug 修复
适用: /workflow bug-fix <description>

Stage 1: reproduce（只读 + terminal）
  → 理解 bug 描述，复现问题
Stage 2: diagnose（只读）
  → 定位根因，分析影响范围
Stage 3: fix（读写）
  → 实施修复
Stage 4: verify（terminal）
  → 编译、运行测试验证修复
```

#### 模板 3: release

```
名称: 发布
适用: /workflow release <version>

Stage 1: check（只读 + terminal）
  → 检查当前状态（未提交变更、测试状态、版本号）
Stage 2: prepare（读写）
  → 更新版本号、生成 changelog
Stage 3: build（terminal）
  → 编译、运行测试
Stage 4: finalize（terminal）
  → git tag、commit
```

#### 模板 4: refactor

```
名称: 重构
适用: /workflow refactor <description>

Stage 1: analyze（只读）
  → 分析依赖关系，确定重构范围和方案
Stage 2: plan（只读）
  → 制定具体步骤（文件拆分/合并/重命名）
Stage 3: execute（读写）
  → 逐文件实施重构
Stage 4: verify（terminal）
  → 编译验证、运行测试
```

### 7.6 用户自定义模板

用户可以在 `~/.qianxun/workflows/` 目录下创建自己的 Workflow 模板：

```
~/.qianxun/workflows/
├── my-workflow.md       # 单文件模板
└── deploy/              # 目录模板
    ├── template.md      # 模板定义
    └── scripts/         # 辅助脚本
```

自定义模板格式：

```markdown
---
id: my-workflow
name: 我的工作流
description: 自定义工作流程
---

## Stage 1: prepare

准备工作。

**可用工具**: read, search, think

请先读取配置文件和模板文件，确认环境就绪。

完成当前阶段后输出 `## Stage Complete: prepare`。

## Stage 2: execute

执行主操作。

**可用工具**: read, write, terminal, search

按以下步骤操作：
1. 第一步...
2. 第二步...

完成当前阶段后输出 `## Stage Complete: execute`。
```

#### 解析规则

```
1. 以 `---` frontmatter 声明 id/name/description
2. 正文中以 `## Stage N: name` 定义阶段
3. 每个阶段中：
   - `**可用工具**` 行解析 allowed_tools
   - 其余文本为 instructions
4. 阶段间顺序 = 在文档中出现的顺序
```

### 7.7 模板加载

```rust
// === agent/workflow.rs

pub struct WorkflowManager {
    /// 内置模板
    builtin: HashMap<WorkflowTemplateId, WorkflowTemplate>,
    /// 用户自定义模板
    custom: HashMap<WorkflowTemplateId, WorkflowTemplate>,
}

impl WorkflowManager {
    pub fn new() -> Self {
        let mut builtin = HashMap::new();
        builtin.insert("code-review".into(), Self::builtin_code_review());
        builtin.insert("bug-fix".into(), Self::builtin_bug_fix());
        builtin.insert("release".into(), Self::builtin_release());
        builtin.insert("refactor".into(), Self::builtin_refactor());
        
        Self { builtin, custom: HashMap::new() }
    }
    
    /// 加载用户自定义模板
    pub fn load_custom(&mut self, path: &Path) -> Result<()> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.path().extension().map_or(false, |e| e == "md") {
                let template = WorkflowTemplate::from_file(entry.path())?;
                self.custom.insert(template.id.clone(), template);
            }
        }
        Ok(())
    }
    
    /// 获取模板（自定义优先）
    pub fn get(&self, id: &str) -> Option<&WorkflowTemplate> {
        self.custom.get(id).or_else(|| self.builtin.get(id))
    }
    
    /// 列出所有可用模板
    pub fn list(&self) -> Vec<&WorkflowTemplate> {
        let mut templates: Vec<_> = self.builtin.values().chain(self.custom.values()).collect();
        templates.sort_by_key(|t| &t.id);
        templates
    }
}
```

---

## 8. 配置

### 8.1 全局配置（config.json）

```json
{
  agent: {
    pattern: "react",              // 默认模式

    plan_and_execute: {
      auto_execute: false,
      max_plan_turns: 20,
      max_execute_turns: 50,
      approval_timeout_sec: 300,   // ACP 模式等待用户确认超时（5 分钟），CLI 模式无限制
    },

    reflective: {
      max_review_rounds: 2,
      review_confidence_threshold: 8,
      only_review_when_tool_used: true,
    },

    workflow: {
      max_stage_turns: 30,         // 每个阶段最大 LLM 轮次
      custom_path: "~/.qianxun/workflows",  // 用户自定义模板路径
    },
  }
}
```

### 8.2 CLI 命令

```
> /pattern              → 显示当前模式
> /pattern react        → 切换到 React
> /pattern plan         → 切换到 Plan-and-Execute
> /pattern reflective   → 切换到 Reflective
> /workflow             → 列出可用 Workflow 模板
> /workflow code-review → 启动代码审查工作流
> /workflow bug-fix "xxx 出现 panic"  → 启动 Bug 修复工作流
```

**CLI 参数**：

```
qx --pattern plan
qx --workflow code-review
qx --workflow bug-fix "登录页面崩溃"
```

### 8.3 ACP 协议

```json
// 创建会话时指定模式
{
  "method": "sessions/new",
  "params": {
    "pattern": { "workflow": "code-review" }
  }
}

// Workflow 阶段通知
{
  "method": "session/stage",
  "params": {
    "session_id": "sess_abc",
    "workflow": "code-review",
    "stage": "analyze",
    "stage_index": 0,
    "total_stages": 3,
    "status": "entered"  // "entered" | "completed"
  }
}
```

### 8.4 AgentConfig 扩展

```rust
// === types.rs

pub struct AgentConfig {
    pub max_turns: u32,
    pub max_retries: u32,
    pub max_tokens: u32,
    pub temperature: f64,
    pub thinking: Option<ThinkingConfig>,
    
    pub pattern: AgentPattern,
    
    // 各模式的子配置
    pub plan_and_execute: PlanAndExecuteConfig,
    pub reflective: ReflectiveConfig,
    pub workflow: WorkflowConfig,
}

pub struct WorkflowConfig {
    pub max_stage_turns: u32,
    pub custom_path: Option<String>,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            max_stage_turns: 30,
            custom_path: Some("~/.qianxun/workflows".into()),
        }
    }
}
```

---

## 9. 状态机

### 9.1 完整状态机

```rust
// === agent/pattern.rs

pub enum PatternState {
    React,
    PlanAndExecute(PlanAndExecuteState),
    Reflective(ReflectiveState),
    Workflow(WorkflowState),
}
```

### 9.2 Plan-and-Execute 状态

```
PlanAndExecuteState:
  ├─ Planning         ← 计划阶段（只读工具）
  ├─ WaitingApproval  ← 等待用户确认（挂起）
  ├─ Executing        ← 执行阶段（全部工具）
  └─ Completed        ← 完成
```

### 9.3 Reflective 状态

```
ReflectiveState:
  ├─ Reacting     ← Phase 1: 标准 React 循环
  ├─ Reviewing    ← Phase 2: 自检
  ├─ Revising     ← 有 issues，修正中
  └─ Completed    ← 完成
```

### 9.4 Workflow 状态

```
WorkflowState:
  ├─ Running { template, current_stage_index }
  │      ← 正在执行第 N 阶段
  │
  ├─ StageComplete { template, stage_index }
  │      ← 当前阶段已完成，准备进入下一阶段
  │
  └─ Completed ← 所有阶段都完成
```

```rust
pub struct WorkflowState {
    pub template: WorkflowTemplate,
    pub current_stage_index: usize,
    pub stage_results: Vec<StageResult>,
}

pub struct StageResult {
    pub stage_name: String,
    pub summary: String,              // 阶段输出摘要
    pub tool_calls: Vec<String>,      // 本阶段调用的工具列表
    pub llm_rounds: u32,              // 本阶段的 LLM 调用轮次
}
```

---

## 10. 依赖清单

```toml
# 不需要新增 crate
# 所有改动都在 qianxun-core/src/agent/ 内部
# 用户自定义模板解析不需要引入 gray-matter 等 crate
#   → 自解析：分割 --- frontmatter + 按 ## Stage N: 分割阶段
```

---

## 11. 测试策略

| 测试类型 | 覆盖 |
|---|---|
| **通用** | |
| 单元测试 | AgentPattern 序列化/反序列化（config 兼容） |
| | |
| **Plan-and-Execute** | |
| 单元测试 | PlanResult 的依赖解析（DAG 验证） |
| 单元测试 | Plan 阶段写工具被拒绝 |
| 集成测试 | 完整流程：计划 → 确认 → 执行 |
| | |
| **Reflective** | |
| 单元测试 | 重试保护（MAX_REVIEW_ROUNDS 上限） |
| 集成测试 | 无 tool_call 时不触发自检 |
| 集成测试 | 自检发现问题后修正（最多 2 轮） |
| | |
| **Workflow** | |
| 单元测试 | 模板解析：内置模板加载 |
| 单元测试 | 用户自定义模板解析（合法 frontmatter + 阶段定义） |
| 单元测试 | 非法模板处理（frontmatter 缺失、阶段重复） |
| 单元测试 | 阶段工具过滤器：analyze 阶段 write 工具被拒绝 |
| 集成测试 | code-review 完整三阶段 |
| 集成测试 | exit_marker 检测：Agent 输出标记后自动跳转下一阶段 |
| 集成测试 | max_stage_turns 超时后强制进入下一阶段 |
| | |
| **CLI** | |
| Cli 测试 | `/pattern` 命令切换模式 |
| Cli 测试 | `/workflow` 列出模板 |
| Cli 测试 | `/workflow code-review` 启动工作流 |
| | |
| **ACP** | |
| ACP 测试 | plan 通知 + decision 响应 |
| ACP 测试 | workflow stage 通知 |

---

## 12. 里程碑建议

| 阶段 | 任务 | 预估 |
|---|---|---|
| **1. 基础设施** | AgentPattern 枚举 + PatternState 状态机 + Config 扩展 | 1 天 |
| **2. 工具权限门控** | ToolCategory 声明 + ToolRegistry::execute_with_filter | 1 天 |
| **3. Plan-and-Execute** | plan.rs 两阶段循环 + PlanResult + 用户确认门控 | 2.5 天 |
| **4. Reflective** | reflect.rs 双轮 + 审查 prompt + 重试保护 | 2 天 |
| **5. Workflow 核心** | WorkflowTemplate 数据结构 + handle_workflow() 循环 | 2 天 |
| **6. 内置模板** | 4 个内置模板定义（code-review / bug-fix / release / refactor） | 1 天 |
| **7. 用户自定义模板** | 文件解析 + ~/.qianxun/workflows/ 加载 | 1.5 天 |
| **8. CLI 集成** | `/pattern` + `/workflow` 命令 + `--workflow` 参数 | 1 天 |
| **9. ACP 扩展** | workflow stage 通知 + 模式查询 | 1 天 |
| **10. 系统提示词** | 各模式的 mode_instructions 注入 | 0.5 天 |
| **11. 测试** | 单元 + 集成 + CLI + ACP | 3 天 |
| **合计** | | **~16.5 天** |
