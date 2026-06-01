---
状态: 生效
适用范围: qianxun-core/src/skills/
最后更新: 2026-06-01
---

# Skills 子系统状态

## 一句话摘要
SkillManager 完整实现：frontmatter 解析、自动匹配、@引用、文件监听、inject 构建。

## 源文件清单
-  — SkillManager（load_all/reload/auto_select/select_by_name/injections）
-  — SkillWatcher（基于 notify crate 的文件变更检测）
-  — skill_read 工具注册

## 当前状态

| 子模块 | 状态 | 说明 |
|--------|------|------|
| load_all | ✅ | 全局 ~/.claude/skills + 项目 skills/ 目录加载 |
| frontmatter 解析 | ✅ | YAML frontmatter + body 分离 |
| auto_select | ✅ | 关键词匹配自动选择 |
| select_by_name | ✅ | @名精确选择 |
| build_catalog_prompt | ✅ | 格式化技能目录 |
| build_injections | ✅ | 构建注入上下文 |
| SkillWatcher | ✅ | notify-based 文件变更检测与重载 |
| skill_read 工具 | ✅ | Agent 可通过工具读取技能内容 |
| README.md skill | 🔧 | 未实现目录作为 skill |
| depends_on/priority | 🔧 | 字段解析，但依赖排序未实现 |

## 已知缺口
- 项目级技能路径未定（.qianxun/skills 还是 .claude/skills）
- 目录作为 skill 未实现
- 依赖排序未实现
