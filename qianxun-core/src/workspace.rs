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
    pub claude_md: Option<String>,
    pub detected_files: Vec<String>,
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
                "CLAUDE.md" => {
                    claude_content = std::fs::read_to_string(entry.path()).ok();
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
                // 检查是否是 workspace
                let is_ws = claude_content.as_ref()
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

            return Some(Workspace {
                root: dir.to_path_buf(),
                project_type,
                claude_md: claude_content,
                detected_files: detected,
            });
        }

        current = dir.parent();
    }

    None
}

/// 构建可注入 system prompt 的工作区上下文字符串。
pub fn build_workspace_context(ws: &Workspace) -> String {
    let mut parts = vec!["## 工作区上下文".to_string()];

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

    if let Some(ref rules) = ws.claude_md {
        // 截取 CLAUDE.md 前 2000 字符避免撑爆 prompt
        let truncated = if rules.len() > 2000 {
            format!("{}...\n[以下内容已截断，原长度 {} 字符]", &rules[..2000], rules.len())
        } else {
            rules.clone()
        };
        parts.push(format!("\n### 项目规则 (CLAUDE.md)\n{truncated}"));
    }

    parts.join("\n")
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
            claude_md: None,
            detected_files: vec!["Cargo.toml".into()],
        };
        let ctx = build_workspace_context(&ws);
        assert!(ctx.contains("/test/project"));
        assert!(ctx.contains("Rust"));
        assert!(ctx.contains("Cargo.toml"));
    }
}
