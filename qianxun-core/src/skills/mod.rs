use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    fn global_skills_dir() -> Option<PathBuf> {
        let home = if cfg!(target_os = "windows") {
            std::env::var("USERPROFILE").ok()
        } else {
            std::env::var("HOME").ok()
        }?;
        Some(PathBuf::from(home).join(".qianxun").join("skills"))
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

        for line in frontmatter.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("name:") {
                name = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("description:") {
                description = val.trim().to_string();
            }
        }

        if name.is_empty() {
            return None;
        }

        Some(Skill {
            metadata: SkillMetadata {
                name: SkillName(name),
                description,
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
            return "（无）".to_string();
        }

        let mut list = String::new();
        for (_source, meta) in &self.skills {
            list.push_str(&format!("  - **{}**", meta.name.as_str()));
            if !meta.description.is_empty() {
                list.push_str(&format!(": {}", meta.description));
            }
            list.push('\n');
        }
        list
    }

    /// 构建注入到 system prompt 的技能目录（含 body 内容）。
    pub fn build_catalog_prompt(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let mut catalog = String::new();
        for (_source, meta) in &self.skills {
            catalog.push_str(&format!("### {}\n", meta.name.as_str()));
            if !meta.description.is_empty() {
                catalog.push_str(&format!("{}\n", meta.description));
            }
            if let Some(body) = self.body_cache.get(meta.name.as_str()) {
                if !body.is_empty() {
                    catalog.push_str(&format!("\n{body}\n"));
                }
            }
            catalog.push('\n');
        }
        catalog
    }

    pub fn available_skills(&self) -> Vec<String> {
        self.skills
            .iter()
            .map(|(_, m)| m.name.as_str().to_string())
            .collect()
    }
}
