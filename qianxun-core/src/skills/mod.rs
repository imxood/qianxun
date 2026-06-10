use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher as NotifyWatcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub mod lifecycle;

#[derive(Debug, Clone)]
pub struct SkillName(String);

impl SkillName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct SkillMetadata {
    pub name: SkillName,
    pub description: String,
    pub trigger_keywords: Vec<String>,
    pub disable_model_invocation: bool,
}

#[derive(Debug, Clone)]
pub enum SkillSource {
    BuiltIn,
    Global,
    ProjectLocal,
}

pub struct Skill {
    pub metadata: SkillMetadata,
    pub source: SkillSource,
    pub directory: std::path::PathBuf,
    pub body: Option<String>,
}

#[derive(Clone)]
pub struct SkillManager {
    skills: Vec<(SkillSource, SkillMetadata)>,
    body_cache: HashMap<String, String>,
}

impl Default for SkillManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillManager {
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            body_cache: HashMap::new(),
        }
    }

    /// 从全局 `~/.qianxun/skills/` 和项目 `.claude/skills/` 加载技能。
    pub fn load_all(project_dir: Option<&Path>) -> Self {
        let mut manager = Self::new();

        if let Some(global_dir) = Self::global_skills_dir() {
            let count = manager.load_from_dir(&global_dir, SkillSource::Global);
            if count > 0 {
                tracing::info!("[skills] loaded {count} skills from {:?}", global_dir);
            }
        }

        if let Some(proj) = project_dir {
            let project_skills = proj.join(".claude").join("skills");
            let count = manager.load_from_dir(&project_skills, SkillSource::ProjectLocal);
            if count > 0 {
                tracing::info!("[skills] loaded {count} skills from {:?}", project_skills);
            }
        }

        manager
    }

    /// 全局技能目录 `~/.qianxun/skills/`。
    pub fn global_skills_dir() -> Option<PathBuf> {
        crate::workspace::qianxun_dir().map(|d| d.join("skills"))
    }

    /// 从目录加载所有 `.md` 技能文件，返回加载数量。
    fn load_from_dir(&mut self, dir: &Path, source: SkillSource) -> usize {
        if !dir.is_dir() {
            return 0;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return 0,
        };

        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            if let Some(skill) = Self::parse_file(&path, source.clone()) {
                let name = skill.metadata.name.as_str().to_string();
                if let Some(body) = &skill.body {
                    self.body_cache.insert(name.clone(), body.clone());
                }
                self.skills.push((source.clone(), skill.metadata));
                count += 1;
            }
        }
        count
    }

    /// 解析技能文件 frontmatter + body。
    fn parse_file(path: &Path, source: SkillSource) -> Option<Skill> {
        let content = std::fs::read_to_string(path).ok()?;
        let content = content.trim();

        // 必须有 `---` frontmatter
        let after_first = content.strip_prefix("---")?;
        let closing = after_first.find("---")?;
        let frontmatter = after_first[..closing].trim();
        let body = after_first[closing + 3..].trim().to_string();

        let mut name = String::new();
        let mut description = String::new();
        let mut trigger_keywords = Vec::new();

        for line in frontmatter.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("name:") {
                name = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("description:") {
                description = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("triggers:") {
                trigger_keywords = val
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }

        if name.is_empty() {
            return None;
        }

        Some(Skill {
            metadata: SkillMetadata {
                name: SkillName(name),
                description,
                trigger_keywords,
                disable_model_invocation: false,
            },
            source,
            directory: path.parent()?.to_path_buf(),
            body: Some(body),
        })
    }

    /// 重新加载技能（清除缓存后重新扫描）。
    pub fn reload(&mut self, project_dir: Option<&Path>) {
        self.skills.clear();
        self.body_cache.clear();

        if let Some(global_dir) = Self::global_skills_dir() {
            self.load_from_dir(&global_dir, SkillSource::Global);
        }
        if let Some(proj) = project_dir {
            let project_skills = proj.join(".claude").join("skills");
            self.load_from_dir(&project_skills, SkillSource::ProjectLocal);
        }
    }

    /// 读取指定技能的 body 内容。
    pub fn read_body(&self, name: &str) -> Option<&str> {
        self.body_cache.get(name).map(|s| s.as_str())
    }

    /// 构建仅含名称和描述的技能列表（用于 `/skills` 展示）。
    pub fn build_skills_list(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut list = String::new();
        for (source, meta) in &self.skills {
            let src_label = match source {
                SkillSource::Global => "全局",
                SkillSource::ProjectLocal => "项目",
                SkillSource::BuiltIn => "内置",
            };
            list.push_str(&format!("  📦 **{}** ({})", meta.name.as_str(), src_label));
            if !meta.description.is_empty() {
                list.push_str(&format!(": {}", meta.description));
            }
            list.push('\n');
        }
        list
    }

    /// 返回技能数量。
    pub fn skill_count(&self) -> usize {
        self.skills.len()
    }

    /// 构建技能目录（Layer 1 — 始终注入 system prompt）。
    /// 仅显示名称、描述和触发词，不含完整 body。
    pub fn build_catalog_prompt(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut catalog = String::from("## 可用技能\n");
        catalog.push_str("当你的需求匹配触发词时，系统会自动注入完整指令。你也可以用 @技能名 手动引用。\n\n");

        for (_source, meta) in &self.skills {
            catalog.push_str(&format!("- **{}**", meta.name.as_str()));
            if !meta.description.is_empty() {
                catalog.push_str(&format!(": {}", meta.description));
            }
            if !meta.trigger_keywords.is_empty() {
                catalog.push_str(&format!("。触发词: {}", meta.trigger_keywords.join("、")));
            } else {
                catalog.push_str(&format!("。手动引用: @{}", meta.name.as_str()));
            }
            catalog.push('\n');
        }
        catalog
    }

    /// 移除外层代码块内容，避免在代码块内的关键词误触发技能匹配。
    fn strip_code_blocks(msg: &str) -> String {
        let mut out = String::with_capacity(msg.len());
        let mut in_block = false;
        for line in msg.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                in_block = !in_block;
                continue;
            }
            if !in_block {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// 自动匹配：根据用户消息中的关键词选择匹配的技能。
    /// - `user_message`: 用户当前输入
    /// - `exclude_names`: 排除列表（最近已注入的 + 手动已选的）
    /// - 返回匹配的技能名称列表
    pub fn auto_select(&self, user_message: &str, exclude_names: &[&str]) -> Vec<String> {
        if self.skills.is_empty() {
            return Vec::new();
        }

        let stripped = Self::strip_code_blocks(user_message);
        let msg_lower = stripped.to_lowercase();
        let mut matched = Vec::new();

        for (_, meta) in &self.skills {
            if exclude_names.contains(&meta.name.as_str()) {
                continue;
            }
            if meta.trigger_keywords.is_empty() {
                continue;
            }
            for kw in &meta.trigger_keywords {
                if msg_lower.contains(&kw.to_lowercase()) {
                    matched.push(meta.name.as_str().to_string());
                    break;
                }
            }
        }

        matched
    }

    /// 通过名称精确选择技能（用于 @技能名 手动引用）。
    pub fn select_by_name(&self, name: &str) -> Option<&SkillMetadata> {
        self.skills
            .iter()
            .map(|(_, meta)| meta)
            .find(|meta| meta.name.as_str() == name)
    }

    /// 构建单技能的注入 body（Layer 2 — 完整指令内容）。
    pub fn build_injection_body(&self, name: &str) -> Option<String> {
        let body = self.body_cache.get(name)?;
        Some(format!(
            "<skill>\n<name>{name}</name>\n{body}\n</skill>",
        ))
    }

    /// 从消息中提取 @技能名 手动引用。
    pub fn extract_manual_mentions(msg: &str) -> Vec<String> {
        msg.split_whitespace()
            .filter_map(|word| {
                if let Some(rest) = word.strip_prefix('@') {
                    let name = rest.trim_end_matches(|c: char| c.is_ascii_punctuation());
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
                None
            })
            .collect()
    }

    /// 构建多个技能的注入（Layer 2），供 Conversation::build_request 使用。
    pub fn build_injections(&self, names: &[String]) -> String {
        let mut result = String::new();
        for name in names {
            if let Some(body) = self.build_injection_body(name) {
                result.push_str(&body);
                result.push('\n');
            }
        }
        result
    }

    pub fn available_skills(&self) -> Vec<String> {
        self.skills
            .iter()
            .map(|(_, m)| m.name.as_str().to_string())
            .collect()
    }
}

/// 监听技能目录文件变更，自动触发 SkillManager::reload()。
pub struct SkillWatcher {
    changed: Arc<AtomicBool>,
    _watcher: Option<RecommendedWatcher>,
}

impl SkillWatcher {
    /// 监听全局 `~/.qianxun/skills/` 和项目 `.claude/skills/` 目录。
    /// 目录不存在时静默跳过，不报错。
    pub fn new(project_dir: Option<&Path>) -> Self {
        let changed = Arc::new(AtomicBool::new(false));
        let changed_clone = changed.clone();

        let watcher_result = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let is_md = event.paths.iter().any(|p| {
                        p.extension().and_then(|e| e.to_str()) == Some("md")
                    });
                    if is_md {
                        changed_clone.store(true, Ordering::SeqCst);
                    }
                }
            },
            Config::default(),
        );

        let mut watcher = match watcher_result {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!("[skill_watcher] failed to create: {e}");
                return Self { changed, _watcher: None };
            }
        };

        // 监听全局技能目录
        if let Some(global_dir) = SkillManager::global_skills_dir() {
            if global_dir.is_dir() {
                if let Some(ref mut w) = watcher {
                    if let Err(e) = w.watch(&global_dir, RecursiveMode::NonRecursive) {
                        tracing::warn!("[skill_watcher] watch {:?} failed: {e}", global_dir);
                    }
                }
            }
        }

        // 监听项目技能目录
        if let Some(proj) = project_dir {
            let project_skills = proj.join(".claude").join("skills");
            if project_skills.is_dir() {
                if let Some(ref mut w) = watcher {
                    if let Err(e) = w.watch(&project_skills, RecursiveMode::NonRecursive) {
                        tracing::warn!("[skill_watcher] watch {:?} failed: {e}", project_skills);
                    }
                }
            }
        }

        Self { changed, _watcher: watcher }
    }

    /// 自上次检查后是否有 `.md` 文件变更（原子读取+重置）。
    pub fn has_changed(&mut self) -> bool {
        self.changed.swap(false, Ordering::AcqRel)
    }
}
