use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SkillName(String);

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

#[allow(dead_code)]
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

    pub fn load_all(_project_dir: Option<&Path>) -> Self {
        // Phase 1: 骨架，实际发现逻辑在后续迭代中实现
        Self::new()
    }

    pub fn reload(&mut self) {
        // Phase 1: 骨架
    }

    pub fn read_body(&mut self, _name: &str) -> Option<&str> {
        // Phase 1: 骨架
        None
    }

    pub fn build_catalog_prompt(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }
        let mut catalog = String::from("可用技能：\n");
        for (_source, meta) in &self.skills {
            catalog.push_str(&format!("- {}: {}\n", meta.name.0, meta.description));
        }
        catalog
    }

    pub fn available_skills(&self) -> Vec<String> {
        self.skills.iter().map(|(_, m)| m.name.0.clone()).collect()
    }
}
