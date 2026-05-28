# Codex Skill 系统分析

> 状态: 完成 | 2026-05-28
> 来源: codex-rs 项目 (E:\git\ai\codex\codex-rs)

---

## 1. 概述

Codex 的 Skill 系统是一套**全自动 + 显式引用**的技能注入机制。加载后，所有启用的 Skill 默认参与候选，通过用户对话中的显式提及或命令分析自动触发。

核心链路：

```
发现 → 加载 → 筛选 → 候选 → 注入
```

---

## 2. Skill 发现与加载

### 2.1 搜索路径（按优先级）

| Scope | 路径 | 说明 |
|-------|------|------|
| System | 内置 bundled skills | 随 Codex 安装，优先最高 |
| Admin | `/etc/codex/skills/` | 管理员全局部署 |
| Repo | `.codex/skills/` | 项目级，从 config layer 获取 |
| User | `~/.agents/skills/`, `$CODEX_HOME/skills/` | 用户级 |

Scope 排序（高→低）：System(0) > Admin(1) > Repo(2) > User(3)

### 2.2 文件扫描

- BFS 遍历每个 root，最大深度 6 层
- 每 root 最多 2000 个目录
- 只识别 `SKILL.md` 文件

### 2.3 解析流程

每个 `SKILL.md` 两步解析：

1. **Frontmatter**（`---` 分隔的 YAML 头部）：
   - 必填: `name`、`description`
   - 可选: `metadata`（扩展字段）

2. **Body**: Markdown 正文，作为技能的完整指令内容

### 2.4 扩展元数据

同目录下的 `agents/openai.yaml` 可选提供：

- `interface`: 接口定义
- `dependencies`: 依赖声明
- `policy`:
  - `allow_implicit_invocation`: 默认 `true`，控制是否允许自动触发
  - `products`: 限制该 Skill 可用的产品列表

---

## 3. 筛选阶段

### 3.1 Config 规则筛选

```rust
SkillConfigRule { selector: Name|Path, enabled: bool }
```

- 支持按 `name` 或 `path` 精确启用/禁用
- 规则来源：User 层 + SessionFlags 层
- `resolve_disabled_skill_paths()` 从名称/路径规则解析出 `disabled_paths: HashSet`

### 3.2 Product 过滤

- `SkillManager` 可以绑定 `restriction_product`
- `filter_skill_load_outcome_for_product()` 只保留匹配 Product 的 Skill

### 3.3 Policy 过滤

- `allow_implicit_invocation: false` 的 Skill 不会被自动触发（但仍可在显式引用时选中）

---

## 4. 候选与触发机制

### 4.1 显式触发（注入前主动选择）

**核心函数**: `collect_explicit_skill_mentions()` (injection.rs)

从用户输入文本中提取 Skill 引用，分两步：

#### Step 1 — 提取 Tool 提及

```rust
fn extract_tool_mentions(text: &str) -> Vec<ToolMention>
```

两种格式：

| 格式 | 示例 | 优先级 |
|------|------|--------|
| Sigil 前缀 | `` `skill-name `` | 高 |
| Markdown 链接 | `[name](skill://path)` | 高 |

使用反引号 `` ` `` 作为 TOOL_MENTION_SIGIL（来自 codex_protocol）。

#### Step 2 — 解析歧义

```rust
fn select_skills_from_mentions(tool_mentions, skills, skill_name_counts) -> Vec<SkillMetadata>
```

- 先尝试 `skill_name_counts` 精确匹配
- 多个 Skill 同名时，只命中路径显式匹配
- 名字无歧义（全局唯一）时直接用名字匹配

#### 完整时序

```
用户输入 → extract_tool_mentions() → select_skills_from_mentions() → build_skill_injections()
```

返回 `Vec<SkillInjection>`，包含所有显式匹配的 Skill。

### 4.2 隐式自动触发（无需用户指定）

**核心函数**: `detect_implicit_skill_invocation_for_command()` (invocation_utils.rs)

在解析 shell 命令时触发：

#### 脚本运行检测

```rust
fn detect_skill_script_run(
    command: &str,
    args: &[String],
    implicit_skills_by_scripts_dir: &...
) -> Option<SkillMetadata>
```

- 识别命令 token（`python3`、`bash`、`node` 等） + 脚本路径
- 沿脚本路径向上遍历，匹配 `implicit_skills_by_scripts_dir`

#### 文档读取检测

```rust
fn detect_skill_doc_read(
    command: &str,
    args: &[String],
    implicit_skills_by_doc_path: &...
) -> Option<SkillMetadata>
```

- 识别 `cat`、`sed`、`head`、`tail` 等读取命令
- 匹配 `implicit_skills_by_doc_path`（在 `finalize_skill_outcome()` 时构建）

### 4.3 隐式索引构建

```rust
fn build_implicit_skill_path_indexes(allowed_skills) -> (ByScriptsDir, ByDocPath)
```

在 `finalize_skill_outcome()` 时预构建：

- `implicit_skills_by_scripts_dir` — 每个 Skill 的 `scripts/` 目录映射
- `implicit_skills_by_doc_path` — 每个 Skill 的 SKILL.md 路径映射

只对 `allowed_skills_for_implicit_invocation()` 返回的 Skill 建索引。

---

## 5. 注入阶段

### 5.1 两层注入结构

#### 第一层 — Skills 目录（developer role）

`available_skills_instructions.rs` → `AvailableSkillsInstructions`

放在 developer role 中，格式为：

```
<skills_instructions_open_tag>
## Skills
...目录...
<skills_instructions_close_tag>
```

内容包含：
- 所有启用的 Skill 列表（名称 + 描述 + 路径）
- 触发使用说明
- 内置 Tool 的使用方式
- Scope 标记

#### 第二层 — 具体 Skill 内容（user role）

`skill_instructions.rs` → `SkillInstructions`

只对**显式选中**的 Skill 注入完整内容：

```
<skill>
<name>skill-name</name>
<path>/path/to/skill</path>
完整 SKILL.md 正文
</skill>
```

放在 user role，和用户消息同层。

### 5.2 Budget 控制

**核心函数**: `build_available_skills()` (render.rs)

| 预算 | 行为 |
|------|------|
| 默认 | context window 的 2% |
| 回退 | 8000 字符 |

**渐进式截断**（scope 优先级由高到低）：

1. 所有 Skill 完整渲染（含描述）
2. 超出预算 → 截断描述文本（保留名称 + 路径）
3. 仍超出 → 从低优先级（User scope）开始整行删除
4. 所有 Skill 行都没了 → 删除根别名行
5. 恢复一部分被截断的 Skill 行（高优先级优先）

---

## 6. 多 Skill 选择

Codex 天然支持选中**多个 Skill**：

- `collect_explicit_skill_mentions()` 返回 `Vec<ToolMention>`
- `select_skills_from_mentions()` 返回 `Vec<SkillMetadata>`
- `build_skill_injections()` 返回 `Vec<SkillInjection>`

每个显式引用的 Skill 都会生成独立的 `<skill>` 注入块。

---

## 7. 分析追踪

Codex 对每次 Skill 注入埋点：

- `select_skills_from_mentions()` 中的 `AnalyticsEvent::SkillMentioned`
- `build_skill_injections()` 结束时统计注入数量和 token 成本
- 每次会话的总注入次数、总 token 开销

---

## 8. 交互示例

### 用户不指定 Skill

1. `available_skills_instructions` 在 system prompt 列出所有启用 Skill 的目录
2. LLM 看到 Skill 列表后，可在回复中 `` `skill-name `` 引用
3. 下一轮交互时，Codex 检测到引用 → 注入完整 `<skill>` 块

### 用户显式指定

```
帮我用 `code-review 审查这段代码 [review](skill://code-review)
```

Codex 立即解析 `code-review` 提及，注入完整内容。

### 自动隐式触发

```
运行 scripts/deploy.sh
```

Codex 检测到脚本路径匹配某个 Skill 的 `scripts/` 目录 → 自动选择该 Skill。

---

## 9. 与千寻的对比

| 能力 | Codex | 千寻（当前） |
|------|-------|-------------|
| Skill 发现 | BFS 扫描 SKILL.md | 待实现 |
| 显式触发 | `` `name `` 或 `[name](skill://path)` | 无 |
| 隐式触发 | 命令/脚本分析 | 无 |
| 多 Skill 选择 | 支持 Vec<> | 无 |
| 预算注入 | context window 2% 渐进截断 | 无 |
| Scope 管理 | System/Admin/Repo/User 四级 | 无 |
| 启用/禁用 | Config 规则 + Product 过滤 | 无 |
| 分析追踪 | 每次注入埋点 | 无 |

千寻当前将所有 Skill 内容无差别注入 system prompt，无选择/触发/追踪机制。

---

## 10. 关键文件索引

| 文件 | 角色 |
|------|------|
| `core-skills/src/loader.rs` | Skill 发现 + 解析 |
| `core-skills/src/model.rs` | 数据模型 + Policy |
| `core-skills/src/config_rules.rs` | 启用/禁用规则 |
| `core-skills/src/manager.rs` | 加载 + 缓存 + 筛选 |
| `core-skills/src/injection.rs` | 显式触发分析 |
| `core-skills/src/invocation_utils.rs` | 隐式触发分析 |
| `core-skills/src/render.rs` | Budget 控制 + 渲染 |
| `core-skills/src/mention_counts.rs` | 名称歧义消解 |
| `core/src/context/skill_instructions.rs` | Skill 内容注入格式 |
| `core/src/context/available_skills_instructions.rs` | Skills 目录注入格式 |
