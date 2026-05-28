use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 检测到的项目类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectType {
    Rust { is_workspace: bool },
    Node,
    Python,
    Generic,
}

/// 工作区信息
#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub project_type: ProjectType,
    /// 项目提示词内容（来自 AGENTS.md 或 CLAUDE.md）
    pub project_instructions: Option<String>,
    pub detected_files: Vec<String>,
}

/// 项目级 .qianxun/config.json 配置
#[derive(Debug, Deserialize, Default)]
struct ProjectConfig {
    /// 指定使用哪个提示词文件："AGENTS.md" 或 "CLAUDE.md"
    /// 未指定时自动检测
    #[serde(rename = "prompt_file")]
    prompt_file: Option<String>,
}

/// 读取 `.qianxun/config.json`，返回 `prompt_file` 设置。
fn read_project_prompt_file(root: &Path) -> Option<String> {
    let config_path = root.join(".qianxun").join("config.json");
    if !config_path.exists() {
        return None;
    }
    let file = std::fs::File::open(config_path).ok()?;
    let reader = json_comments::StripComments::new(file);
    let cfg: ProjectConfig = serde_json::from_reader(reader).ok()?;
    cfg.prompt_file
}

/// 确定项目提示词内容。
///
/// 优先级：
/// 1. `.qianxun/config.json` 中指定 `prompt_file` → 读取该文件
/// 2. 自动检测：两者都存在时 `AGENTS.md` 优先
fn resolve_project_instructions(root: &Path, agents_content: &Option<String>, claude_content: &Option<String>) -> Option<String> {
    // Priority 1: config 指定 prompt_file
    if let Some(file) = read_project_prompt_file(root) {
        let path = root.join(&file);
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
        }
        return None;
    }

    // Priority 2: auto-detect
    if agents_content.is_some() {
        return agents_content.clone();
    }
    claude_content.clone()
}

/// 直接从给定路径创建 Workspace（不向上查找）。
/// 仅扫描该目录，检测项目类型和标志文件。
pub fn workspace_from_root(root: &Path) -> Workspace {
    let root = if root.is_relative() {
        std::env::current_dir().map(|p| p.join(root)).unwrap_or(root.to_path_buf())
    } else {
        root.to_path_buf()
    };

    let mut detected = Vec::new();
    let mut has_cargo_toml = false;
    let mut has_package_json = false;
    let mut has_python = false;
    let mut claude_content = None;
    let mut agents_content = None;

    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            match name.as_str() {
                "Cargo.toml" => {
                    has_cargo_toml = true;
                    detected.push(name);
                }
                "package.json" => {
                    has_package_json = true;
                    detected.push(name);
                }
                "AGENTS.md" | "CLAUDE.md" => {
                    let content = std::fs::read_to_string(entry.path()).ok();
                    match name.as_str() {
                        "AGENTS.md" => agents_content = content,
                        "CLAUDE.md" => claude_content = content,
                        _ => {}
                    }
                    detected.push(name);
                }
                "pyproject.toml" | "setup.py" | "requirements.txt" => {
                    has_python = true;
                    detected.push(name);
                }
                _ => {}
            }
        }
    }

    let project_type = if has_cargo_toml {
        let is_ws = claude_content.as_ref()
            .map(|c| c.contains("[workspace]"))
            .unwrap_or(false)
            || agents_content.as_ref()
                .map(|c| c.contains("[workspace]"))
                .unwrap_or(false)
            || std::fs::read_to_string(root.join("Cargo.toml"))
                .map(|c| c.contains("[workspace]"))
                .unwrap_or(false);
        ProjectType::Rust { is_workspace: is_ws }
    } else if has_package_json {
        ProjectType::Node
    } else if has_python {
        ProjectType::Python
    } else {
        ProjectType::Generic
    };

    let project_instructions = resolve_project_instructions(&root, &agents_content, &claude_content);

    Workspace {
        root,
        project_type,
        project_instructions,
        detected_files: detected,
    }
}

/// 从指定目录向上查找项目标志文件，最多 3 层父目录。
pub fn detect_workspace(cwd: &Path) -> Option<Workspace> {
    let cwd = if cwd.is_relative() {
        std::env::current_dir().ok().map(|p| p.join(cwd)).unwrap_or(cwd.to_path_buf())
    } else {
        cwd.to_path_buf()
    };

    let mut current = Some(cwd.as_path());
    for _ in 0..3 {
        let dir = match current {
            Some(d) => d,
            None => break,
        };

        let mut detected = Vec::new();
        let mut has_cargo_toml = false;
        let mut has_package_json = false;
        let mut has_python = false;
        let mut claude_content = None;
        let mut agents_content = None;

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => {
                current = dir.parent();
                continue;
            }
        };

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            match name.as_str() {
                "Cargo.toml" => {
                    has_cargo_toml = true;
                    detected.push(name);
                }
                "package.json" => {
                    has_package_json = true;
                    detected.push(name);
                }
                "AGENTS.md" | "CLAUDE.md" => {
                    let content = std::fs::read_to_string(entry.path()).ok();
                    match name.as_str() {
                        "AGENTS.md" => agents_content = content,
                        "CLAUDE.md" => claude_content = content,
                        _ => {}
                    }
                    detected.push(name);
                }
                "pyproject.toml" | "setup.py" | "requirements.txt" => {
                    has_python = true;
                    detected.push(name);
                }
                _ => {}
            }
        }

        if !detected.is_empty() {
            let project_type = if has_cargo_toml {
                let is_ws = claude_content.as_ref()
                    .map(|c| c.contains("[workspace]"))
                    .unwrap_or(false)
                    || agents_content.as_ref()
                        .map(|c| c.contains("[workspace]"))
                        .unwrap_or(false)
                    || std::fs::read_to_string(dir.join("Cargo.toml"))
                        .map(|c| c.contains("[workspace]"))
                        .unwrap_or(false);
                ProjectType::Rust { is_workspace: is_ws }
            } else if has_package_json {
                ProjectType::Node
            } else if has_python {
                ProjectType::Python
            } else {
                ProjectType::Generic
            };

            let project_instructions = resolve_project_instructions(dir, &agents_content, &claude_content);

            return Some(Workspace {
                root: dir.to_path_buf(),
                project_type,
                project_instructions,
                detected_files: detected,
            });
        }

        current = dir.parent();
    }

    None
}

/// 构建可注入 system prompt 的工作区上下文字符串。
pub fn build_workspace_context(ws: &Workspace) -> String {
    let mut parts = vec!["## 工作区上下文\n这是你的工作区，所有 `execute_command` 命令默认在此目录下执行。".to_string()];

    parts.push(format!("- 项目路径: {}", ws.root.display()));

    let type_str = match &ws.project_type {
        ProjectType::Rust { is_workspace: true } => "Rust (workspace)",
        ProjectType::Rust { is_workspace: false } => "Rust",
        ProjectType::Node => "Node.js",
        ProjectType::Python => "Python",
        ProjectType::Generic => "通用",
    };
    parts.push(format!("- 项目类型: {type_str}"));

    if !ws.detected_files.is_empty() {
        parts.push(format!("- 关键文件: {}", ws.detected_files.join(", ")));
    }

    if let Some(ref rules) = ws.project_instructions {
        let truncated = if rules.len() > 2000 {
            format!("{}...\n[以下内容已截断，原长度 {} 字符]", &rules[..2000], rules.len())
        } else {
            rules.clone()
        };
        parts.push(format!("\n### 项目规则\n{truncated}"));
    }

    parts.join("\n")
}

/// 读取全局用户指令 `~/.qianxun/AGENTS.md`。
/// 截断到 4000 字符，文件不存在或为空时返回 None。
pub fn read_global_agents_md() -> Option<String> {
    let home = if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE").ok()
    } else {
        std::env::var("HOME").ok()
    }?;
    let path = PathBuf::from(home).join(".qianxun").join("AGENTS.md");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let content = content.trim().to_string();
    if content.is_empty() {
        return None;
    }
    if content.len() > 4000 {
        let end = (0..=4000).rev().find(|&i| content.is_char_boundary(i)).unwrap_or(0);
        Some(format!("{}...\n[以下内容已截断，原长度 {} 字符]", &content[..end], content.len()))
    } else {
        Some(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_none_for_empty_dir() {
        let dir = std::env::temp_dir().join("qianxun_test_empty");
        let _ = std::fs::create_dir_all(&dir);
        let result = detect_workspace(&dir);
        assert!(result.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_rust_project() {
        let dir = std::env::temp_dir().join("qianxun_test_rust");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").ok();
        let result = detect_workspace(&dir).unwrap();
        assert_eq!(result.project_type, ProjectType::Rust { is_workspace: false });
        assert!(result.detected_files.contains(&"Cargo.toml".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_detect_python_project() {
        let dir = std::env::temp_dir().join("qianxun_test_python");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("pyproject.toml"), "[project]\nname = \"test\"\n").ok();
        let result = detect_workspace(&dir).unwrap();
        assert_eq!(result.project_type, ProjectType::Python);
        assert!(result.detected_files.contains(&"pyproject.toml".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_context_contains_path() {
        let ws = Workspace {
            root: PathBuf::from("/test/project"),
            project_type: ProjectType::Rust { is_workspace: false },
            project_instructions: None,
            detected_files: vec!["Cargo.toml".into()],
        };
        let ctx = build_workspace_context(&ws);
        assert!(ctx.contains("/test/project"));
        assert!(ctx.contains("Rust"));
        assert!(ctx.contains("Cargo.toml"));
    }

    #[test]
    fn test_resolve_agents_preferred() {
        let dir = std::env::temp_dir().join("qianxun_test_agents_first");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").ok();
        std::fs::write(dir.join("AGENTS.md"), "# AGENTS rules").ok();
        std::fs::write(dir.join("CLAUDE.md"), "# CLAUDE rules").ok();

        let ws = workspace_from_root(&dir);
        assert_eq!(ws.project_instructions.as_deref(), Some("# AGENTS rules"));
        assert!(ws.detected_files.contains(&"AGENTS.md".to_string()));
        assert!(ws.detected_files.contains(&"CLAUDE.md".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_prompt_file_override() {
        let dir = std::env::temp_dir().join("qianxun_test_cfg_override");
        let _ = std::fs::create_dir_all(&dir);
        let qx_dir = dir.join(".qianxun");
        let _ = std::fs::create_dir_all(&qx_dir);
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").ok();
        std::fs::write(dir.join("AGENTS.md"), "# AGENTS rules").ok();
        std::fs::write(dir.join("CLAUDE.md"), "# CLAUDE rules").ok();
        std::fs::write(qx_dir.join("config.json"), r#"{"prompt_file": "CLAUDE.md"}"#).ok();

        let ws = workspace_from_root(&dir);
        assert_eq!(ws.project_instructions.as_deref(), Some("# CLAUDE rules"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
