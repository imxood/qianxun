use serde::Deserialize;
use std::path::{Path, PathBuf};

/// 项目根信息
#[derive(Debug, Clone)]
pub struct ProjectRoot {
    pub root: PathBuf,
    /// 项目提示词内容（来自 AGENTS.md 或 CLAUDE.md）
    pub project_instructions: Option<String>,
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
fn resolve_project_instructions(root: &Path) -> Option<String> {
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

    // Priority 2: auto-detect — AGENTS.md > CLAUDE.md
    let agents = root.join("AGENTS.md");
    let claude = root.join("CLAUDE.md");

    if agents.exists() {
        return std::fs::read_to_string(&agents).ok()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
    }
    if claude.exists() {
        return std::fs::read_to_string(&claude).ok()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
    }

    None
}

/// 从 cwd 向上查找 `.qianxun/` 目录来确定项目根。
///
/// 查找范围：最多向上 10 层，到达文件系统根为止。
/// 找到后设置 current_dir 到项目根（调用者负责）。
pub fn find_project_root(cwd: &Path) -> Option<ProjectRoot> {
    let cwd = if cwd.is_relative() {
        std::env::current_dir()
            .ok()
            .map(|p| p.join(cwd))
            .unwrap_or(cwd.to_path_buf())
    } else {
        cwd.to_path_buf()
    };

    let mut current = Some(cwd.as_path());
    for _ in 0..10 {
        let dir = match current {
            Some(d) => d,
            None => break,
        };

        if dir.join(".qianxun").is_dir() {
            let instructions = resolve_project_instructions(dir);
            return Some(ProjectRoot {
                root: dir.to_path_buf(),
                project_instructions: instructions,
            });
        }

        current = dir.parent();
    }

    None
}

/// 从已知路径创建项目根（不查找）。
///
/// 用于用户显式指定 `-w` 参数的场景。
pub fn project_root_from(path: &Path) -> ProjectRoot {
    let root = if path.is_relative() {
        std::env::current_dir()
            .map(|p| p.join(path))
            .unwrap_or(path.to_path_buf())
    } else {
        path.to_path_buf()
    };

    let instructions = resolve_project_instructions(&root);

    ProjectRoot {
        root,
        project_instructions: instructions,
    }
}

/// 构建可注入 system prompt 的项目上下文字符串。
///
/// 包含 Agent 自己无法直接读取的信息：
///   - 项目根路径（.qianxun/ 所在目录）
///   - 当前工作目录（用户启动 qx 的目录，Agent 在此目录下执行文件操作）
///   - 项目规则（CLAUDE.md / AGENTS.md 内容）
///
/// Agent 的文件操作路径相对于当前工作目录。
/// 项目类型（Rust/Node/Python）由 Agent 通过文件读取自行发现。
pub fn build_project_context(root: &ProjectRoot) -> String {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| String::from("未知"));

    let mut parts = vec![
        "## 项目上下文".to_string(),
        format!("- 项目根路径: {}", root.root.display()),
        format!("- 当前工作目录: {cwd}"),
    ];

    if let Some(ref rules) = root.project_instructions {
        let truncated = if rules.len() > 2000 {
            format!(
                "{}...\n[以下内容已截断，原长度 {} 字符]",
                &rules[..rules.char_indices().nth(2000).map(|(i, _)| i).unwrap_or(rules.len())],
                rules.len()
            )
        } else {
            rules.clone()
        };
        parts.push(format!("\n### 项目规则\n{truncated}"));
    }

    parts.join("\n")
}

/// 获取用户 home 目录路径。
pub fn home_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    } else {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

/// 获取 `~/.qianxun` 配置目录路径。
pub fn qianxun_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".qianxun"))
}

/// 读取全局用户指令 `~/.qianxun/AGENTS.md`。
/// 截断到 4000 字符，文件不存在或为空时返回 None。
pub fn read_global_agents_md() -> Option<String> {
    let path = qianxun_dir()?.join("AGENTS.md");
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    let content = content.trim().to_string();
    if content.is_empty() {
        return None;
    }
    if content.len() > 4000 {
        let end = (0..=4000)
            .rev()
            .find(|&i| content.is_char_boundary(i))
            .unwrap_or(0);
        Some(format!(
            "{}...\n[以下内容已截断，原长度 {} 字符]",
            &content[..end],
            content.len()
        ))
    } else {
        Some(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_qianxun_dir() {
        let dir = std::env::temp_dir().join("qianxun_test_qx");
        let _ = std::fs::create_dir_all(dir.join(".qianxun"));
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").ok();
        let result = find_project_root(&dir).unwrap();
        assert_eq!(result.root, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_in_parent_dir() {
        let dir = std::env::temp_dir().join("qianxun_test_parent");
        let _ = std::fs::create_dir_all(dir.join("subdir").join("deep"));
        let _ = std::fs::create_dir_all(dir.join(".qianxun"));
        let result = find_project_root(&dir.join("subdir").join("deep")).unwrap();
        assert_eq!(result.root, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_context_contains_path_and_cwd() {
        let root = ProjectRoot {
            root: PathBuf::from("/test/project"),
            project_instructions: None,
        };
        let ctx = build_project_context(&root);
        assert!(ctx.contains("/test/project"));
        assert!(ctx.contains("当前工作目录"));
    }

    #[test]
    fn test_build_context_with_instructions() {
        let root = ProjectRoot {
            root: PathBuf::from("/test/project"),
            project_instructions: Some("# Rust rules".into()),
        };
        let ctx = build_project_context(&root);
        assert!(ctx.contains("Rust rules"));
    }

    #[test]
    fn test_resolve_agents_preferred() {
        let dir = std::env::temp_dir().join("qianxun_test_agents_first");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join(".qianxun"));
        std::fs::write(dir.join("AGENTS.md"), "# AGENTS rules").ok();
        std::fs::write(dir.join("CLAUDE.md"), "# CLAUDE rules").ok();

        let root = project_root_from(&dir);
        assert_eq!(root.project_instructions.as_deref(), Some("# AGENTS rules"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_prompt_file_override() {
        let dir = std::env::temp_dir().join("qianxun_test_cfg_override");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(dir.join(".qianxun"));
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"test\"\n").ok();
        std::fs::write(dir.join("AGENTS.md"), "# AGENTS rules").ok();
        std::fs::write(dir.join("CLAUDE.md"), "# CLAUDE rules").ok();
        std::fs::write(
            dir.join(".qianxun").join("config.json"),
            r#"{"prompt_file": "CLAUDE.md"}"#,
        )
        .ok();

        let root = project_root_from(&dir);
        assert_eq!(root.project_instructions.as_deref(), Some("# CLAUDE rules"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
