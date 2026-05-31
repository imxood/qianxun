# 千寻 Skill 系统设计

> 版本: 0.2 | 更新: 2026-06-01 | 状态: 已实现
>
> SkillManager + SkillWatcher（文件监视）+ skill_read 工具已注册

---

## 1. 设计目标

### 核心理念

> **Skill 是以 MD 格式编写的结构化知识包，告诉 Agent 如何完成特定领域任务。**

| 目标 | 说明 |
|---|---|
| **纯文本格式** | 基于 Markdown + frontmatter，可读可写可 git 管理 |
| **即写即用** | 不需要编译/注册，放入 skills 目录即可生效 |
| **按需注入** | 不是所有 skill 都注入系统提示词，根据任务相关性选择 |
| **项目级 + 用户级** | 项目 .qianxun/skills/ 和全局 ~/.qianxun/skills/ 两层 |
| **可共享** | 一个 .md 文件即可分享给他人或项目 |

### 非目标

- 编译型 skill（.wasm / 动态库）— Phase 5 评估
- Skill 市场/包管理器 — 未来可能的生态，但非核心
- Skill 依赖版本解析 — 只做简单的 "depends_on" 声明

---

## 2. Skill 格式规范

### 2.1 文件结构

```
my-skill/
├── README.md          # 技能的完整正文
├── assets/            # 技能引用的图片、模板等资源
│   └── template.rs
└── examples/          # 使用示例（可选）
    └── basic.md
```

单文件 skill 只用一个 `.md` 文件，不需要目录。

### 2.2 Frontmatter 规范

```markdown
---
name: egui-skills           # 技能唯一标识，kebab-case
description: egui/eframe 0.34.1 开发技能，覆盖 App 入口、布局、组件库、数据可视化等
version: 1.2.0              # 语义化版本
author: imxood               # 作者（可选）
trigger: when user writes egui code, uses egui widgets, or asks about egui layout
triggers:                    # 触发条件列表（替代 trigger 的多行版本）
  - 用户写 egui/eframe 代码
  - 用户使用 Window/Panel/ScrollArea 等 egui 组件
  - 用户问 egui 布局/性能/状态管理问题
depends_on: []              # 依赖的其他 skill 名称（可选）
project_only: false          # 是否仅项目有效（默认 false，也适用于全局）
priority: normal             # injection 优先级: low / normal / high（默认 normal）
---

正文内容...
```

### 2.3 Frontmatter 字段定义

| 字段 | 必填 | 类型 | 说明 |
|---|---|---|---|
| `name` | ✅ | string | kebab-case 唯一标识，全局唯一 |
| `description` | ✅ | string | 一行摘要，用于路由和注入决策 |
| `version` | ✅ | semver | `major.minor.patch`，用于更新检测 |
| `author` | ❌ | string | 技能作者 |
| `trigger` | ⚠️ | string | 触发条件描述（与 triggers 二选一） |
| `triggers` | ⚠️ | string[] | 触发条件列表（与 trigger 二选一） |
| `depends_on` | ❌ | string[] | 依赖的 skill name 列表 |
| `project_only` | ❌ | bool | 默认 false。true 表示此 skill 只对特定项目有效 |
| `priority` | ❌ | enum | `low` / `normal` / `high`，影响注入优先级 |

### 2.4 正文结构建议

正文没有强制结构，但建议遵循以下组织：

```markdown
## 概述

技能解决什么问题，什么场景下使用。

## 核心概念

领域术语、关键模式、架构决策。

## 使用指南

技能涵盖的主要操作场景。

### 场景一：XXX

具体指导、代码示例、注意事项。

### 场景二：XXX

## 注意事项

常见陷阱、边界情况、已知限制。

## 参考

链接到外部文档、论文、工具。
```

---

## 3. 加载策略

### 3.1 搜索路径

Skill 在两个路径下搜索，按**优先级从高到低**合并：

```
搜索顺序（高 → 低）：
  1. {workspace_root}/.qianxun/skills/*.md        # 项目级 skill
     {workspace_root}/.qianxun/skills/*/README.md
  2. ~/.qianxun/skills/*.md                      # 用户级全局 skill
     ~/.qianxun/skills/*/README.md

合并规则：
  - 相同 name 的 skill，高优先级覆盖低优先级
  - 所以项目级 skill 可以覆盖用户级同名 skill
```

### 3.2 文件格式检测

```
目录 /path/to/skills/
├── my-skill.md              → 单文件 skill，name = "my-skill"
├── my-skill/                → 多文件 skill，name = "my-skill"
│   ├── README.md            → 正文入口（必需）
│   ├── assets/
│   └── examples/
└── another-skill.md
```

### 3.3 加载流程

```
SkillManager::load_all()
  │
  ├─ for each search_path:
  │     │
  │     ├─ 扫描 *.md 文件
  │     │   → 解析 frontmatter
  │     │   → 验证必填字段
  │     │   → 存入 HashMap<name, Skill>
  │     │
  │     └─ 扫描子目录（含 README.md 的）
  │         → 同上
  │
  ├─ 合并各路径（高优先级覆盖低优先级）
  │
  ├─ 解析 depends_on（DAG 验证）
  │   → 检查循环依赖
  │   → 检查缺失依赖
  │
  └─ 返回 Map<String, Skill>
```

### 3.4 Skill 数据结构

```rust
// === skills/types.rs

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,               // kebab-case 标识
    pub description: String,        // 一行摘要
    pub version: String,            // semver
    pub author: Option<String>,
    pub trigger: Option<String>,    // 与 triggers 互相排斥
    pub triggers: Vec<String>,      // 触发条件列表
    pub depends_on: Vec<String>,
    pub project_only: bool,
    pub priority: SkillPriority,    // Low | Normal | High
    pub body: String,               // README.md 正文（不含 frontmatter）
    pub source: SkillSource,        // Project | Global
    pub file_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SkillPriority {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SkillSource {
    Project,
    Global,
}
```

---

## 4. 注入策略

### 4.1 注入时机

Skill 内容在 `build_system_prompt()` 时注入，与 memory context 并列：

```
system_prompt(BASE)
  + "\n\n" + memory_context
  + "\n\n" + skills_catalog      ← 所有 skill 的目录
  + "\n\n" + skill_injections    ← 高优先级 + 匹配的 skill 正文
```

### 4.2 四层注入模型

不是所有 skill 都注入全文，分为四层：

```
Layer 1: 目录（全部 skill）    → 始终注入
  格式：## 可用技能\n- egui-skills: egui 开发指南\n- bevy-skills: Bevy 游戏引擎
  
Layer 2: 正文（高优先级 skill）→ 始终注入
  priority=high 的 skill 正文全量注入
  
Layer 3: 正文（匹配当前任务的 skill）→ 条件注入
  通过 trigger/triggers 匹配用户当前输入
  匹配方式：关键词重叠（用户 query 中的词与 trigger 中的词）
  
Layer 4: 正文（按需）         → Agent 主动调用 skill_read(name)
  Agent 可以在对话中主动调用 skill_read("bevy-skills") 读取正文
```

### 4.3 正文注入优先级

```
         ┌──────────────────────────────┐
         │ 系统提示词 token 预算        │
         │                              │
         │  Layer 1: 目录（~200 tokens）│ ← 始终
         │  Layer 2: 高优先级 skill     │ ← 始终（可能 0 个）
         │  Layer 3: 匹配当前任务       │ ← 条件
         │  Layer 4: (按需)             │ ← Agent 主动
         └──────────────────────────────┘
```

### 4.4 Agent 主动读取

Agent 可以通过 `skill_read` 工具主动读取 skill 正文：

```rust
/// skill_read 工具
pub struct SkillReadTool {
    manager: Arc<SkillManager>,
}

impl AgentTool for SkillReadTool {
    fn name(&self) -> &str { "skill_read" }
    
    fn description(&self) -> &str {
        "读取已加载技能的完整正文。当需要详细了解某个技能的使用方法和规则时调用。"
    }
    
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "技能名称" }
            },
            "required": ["name"]
        })
    }
    
    async fn execute(&self, arguments: Value) -> Result<ToolOutput, ToolError> {
        let name = arguments["name"].as_str().ok_or(ToolError::InvalidArgs)?;
        let body = self.manager.read_body(name)?;
        Ok(ToolOutput::Text(body))
    }
}
```

---

## 5. 更新检测

### 5.1 版本检查

每次 `load_all()` 时检测：

```
1. 解析每个 skill 的 version（semver）
2. 与上次加载的版本比较
3. 如果 version 变化：
   → 重新加载
   → 记录变更日志
   → 如果是 Daemon 模式，通知关联的活跃 session
```

### 5.2 文件监视（Phase 3+）

```rust
// 使用 notify crate 监听 skills 目录变化
let watcher = notify::RecommendedWatcher::new(move |event| {
    match event.kind {
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {
            // 防抖 500ms 后重新加载
            skill_manager.reload();
        }
        _ => {}
    }
})?;
```

---

## 6. 与 ToolRegistry 的关系

```
Skill 系统与工具系统的关系：

SkillManager                  ToolRegistry
────────────                  ────────────
加载 .md 文件                 注册 AgentTool 实现
解析 frontmatter              按 name 调度
管理注入策略                   execution_async()
提供 skill_read 工具           └─ builtin → MCP → Skill（未来）
├─ 不是 AgentTool
└─ 不通过 ToolRegistry 调用
```

Skill 系统**不是**工具系统的第四层。Skill 给 Agent 提供**知识上下文**，不是可调用的函数。当前 Phase 3 不实现 "skill 作为工具"的机制，留到 Phase 5 评估（即 WASM 编译型 skill）。

---

## 7. 配置格式

```json
{
  skills: {
    auto_load: true,                    // 启动时自动加载
    watch_files: true,                  // 监听文件变化自动 reload
    max_injection_tokens: 2000,         // skill 正文注入最大 token 数
    paths: [
      "~/.qianxun/skills",              // 全局路径
    ],
    disable_list: ["obsolete-skill"],   // 禁用列表
    // project skill 路径由工作区自动检测（.qianxun/skills/）
  }
}
```

---

## 8. 示例 Skill

```markdown
---
name: rust-tips
description: Rust 开发技巧和常见模式
version: 1.0.0
author: community
triggers:
  - 用户写 Rust 代码
  - 用户问 Rust 所有权/生命周期/错误处理
  - 用户提到 unsafe/async/trait
priority: normal
---

## 概述

Rust 开发中的常见模式、陷阱和最佳实践。

## 常用模式

### 1. 错误处理

使用 thiserror 定义错误类型，anyhow 在 binary 层传播：

```rust
#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("配置文件未找到: {0}")]
    NotFound(String),
    #[error("配置解析失败: {0}")]
    ParseFailed(#[from] serde_json::Error),
}
```

### 2. 异步测试

不要在 async 测试中用 `block_on`：

```rust
#[tokio::test]
async fn test_fetch_data() {
    let result = fetch_data().await;
    assert!(result.is_ok());
}
```

## 注意事项

- 避免在 async 函数中持有 std::sync::MutexGuard 跨越 .await
- 使用 tokio::task::spawn_blocking 处理 CPU 密集型任务
```
---

## 9. 依赖清单

```toml
# qianxun-core
# 不需要新增 crate

# 已有依赖：
# - serde（frontmatter 解析）
# - serde_json
# - notify（文件监视，已在 workspace 中）
# - regex（trigger 匹配）

# frontmatter 解析不需要引入 gray-matter 等 crate
# Markdown frontmatter 格式简单，手动解析：
#   1. 检查文件头是否以 --- 开头
#   2. 读取到下一个 --- 为止
#   3. serde_yaml 或 serde_json（通过 json_comments 剥离注释）解析 frontmatter 块
```

### 9.1 Frontmatter 解析策略

| 方案 | 评价 |
|---|---|
| 自解析（字符串分割） | 简单、零依赖，足以覆盖 skill .md 的使用场景 |
| `serde_yaml` | ✅ 如果引入，与 `serde_json` 二选一 |
| `gray-matter` crate | ❌ 过度包装，GitHub stars 少 |

**建议**：先用自解析（分割 `---` 块 + 带注释 JSON 解析），如果 frontmatter 场景复杂化再评估 `serde_yaml`。

> **与 Workflow 模板共享**：Workflow 的用户自定义模板（`agent-pattern-design.md` §7.6）也使用相同的 `---` frontmatter 格式。两个模块应提取共享的 `frontmatter::parse()` 工具函数到 `qianxun-core`，避免重复实现。

---

## 10. 测试策略

| 测试类型 | 覆盖 |
|---|---|
| Frontmatter 解析 | 合法/非法 frontmatter、缺失字段、空 body |
| 路径合并 | 项目级覆盖全局级、同名 skill 优先级 |
| DAG 验证 | 正常依赖、循环依赖检测、缺失依赖 |
| Trigger 匹配 | 关键词重叠、全量匹配、部分匹配 |
| 文件监视 | 创建/修改/删除文件后的 reload 行为 |
| 注入 token 预算 | 超过 max_injection_tokens 的截断行为 |

---

## 11. 里程碑建议

| 阶段 | 任务 | 预估 |
|---|---|---|
| **1. 核心解析** | Frontmatter 解析 + 正文提取 | 1.5 天 |
| **2. 加载器** | 双层路径扫描 + 合并 + DAG 验证 | 1.5 天 |
| **3. 注入策略** | 四层注入 + token 预算控制 | 1 天 |
| **4. skill_read 工具** | AgentTool 实现 + 注册 | 0.5 天 |
| **5. 文件监视** | hot-reload + 防抖 | 1 天 |
| **6. 集成测试** | 端到端加载 + 注入 + 读取 | 1.5 天 |
| **合计** | | **~7 天** |
