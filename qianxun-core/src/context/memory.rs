use super::{ContextDocument, ContextProvider};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

/// 基于文件系统的记忆管理器。
///
/// 每条记忆是一个带有 YAML frontmatter 的 `.md` 文件：
/// `~/.qianxun/memory/<project_hash>/<timestamp>.md`
pub struct MemoryManager {
    base_dir: PathBuf,
    project_hash: String,
    recent_count: usize,
}

impl MemoryManager {
    /// 创建新的 MemoryManager。
    ///
    /// * `base_dir` — 记忆存储的根目录（如 `~/.qianxun/memory`）
    /// * `project_root` — 工作区根路径，用于生成项目哈希
    /// * `recent_count` — `build_context()` 返回的最近记忆条数
    pub fn new(base_dir: PathBuf, project_root: &Path, recent_count: usize) -> Self {
        let hash = Self::project_hash(project_root);
        Self {
            base_dir,
            project_hash: hash,
            recent_count,
        }
    }

    /// 计算项目根的确定性哈希（16 字符十六进制）。
    fn project_hash(root: &Path) -> String {
        let canonical = root
            .canonicalize()
            .unwrap_or_else(|_| root.to_path_buf());
        let mut hasher = DefaultHasher::new();
        canonical.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// 返回此项目的记忆存储目录。
    fn memory_dir(&self) -> PathBuf {
        self.base_dir.join(&self.project_hash)
    }

    /// 通过文件名（时间戳）读取特定记忆文件，返回正文内容。
    pub fn read(&self, name: &str) -> Option<String> {
        let path = self.memory_dir().join(name);
        let content = fs::read_to_string(path).ok()?;
        Self::parse_body(&content)
    }

    /// 写入原始内容到指定名称的记忆文件（不经过 frontmatter）。
    pub fn write(&self, name: &str, content: &str) {
        let dir = self.memory_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            tracing::warn!("Failed to create memory dir {}: {e}", dir.display());
            return;
        }
        let path = dir.join(name);
        if let Err(e) = fs::write(&path, content) {
            tracing::warn!("Failed to write memory file {}: {e}", path.display());
        }
    }

    /// 写入一条带有 frontmatter 的结构化记忆。
    ///
    /// 文件名自动生成为 `<timestamp>.md`。
    pub fn write_memory(&self, summary: &str, tags: &[&str], body: &str) {
        let now = chrono::Utc::now();
        let timestamp = now.format("%Y%m%dT%H%M%S%3f").to_string();
        let summary = if summary.len() > 200 {
            &summary[..200]
        } else {
            summary
        };
        // 格式化标签
        let tags_str = if tags.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = tags.iter().map(|t| format!("\"{}\"", t.replace('"', r#"\""#))).collect();
            format!("[{}]", items.join(", "))
        };
        let content = format!(
            "---\ndate: {}\nsummary: {}\ntags: {}\n---\n\n{}",
            now.format("%Y-%m-%dT%H:%M:%SZ"),
            summary,
            tags_str,
            body
        );
        self.write(&format!("{timestamp}.md"), &content);
    }

    /// 构建最近 N 条记忆的格式化上下文文本块。
    ///
    /// 返回可用于注入 system prompt 的 Markdown 文本，每条格式：
    /// ```markdown
    /// ## 记忆：<summary>
    /// - 日期：<date>
    /// - 标签：<tags>
    ///
    /// <body>
    /// ```
    pub fn build_context(&self) -> String {
        let files = self.list_memory_files();
        if files.is_empty() {
            return String::new();
        }

        let mut entries: Vec<String> = Vec::with_capacity(files.len().min(self.recent_count));
        for path in files.iter().take(self.recent_count) {
            if let Some((meta, body)) = Self::parse_memory_file(path) {
                let summary = meta.get("summary").map(|s| s.as_str()).unwrap_or("");
                let date = meta.get("date").map(|d| d.as_str()).unwrap_or("");
                let tags = meta.get("tags").map(|t| t.as_str()).unwrap_or("[]");
                let body_truncated = if body.len() > 500 {
                    let truncated: String = body.chars().take(500).collect();
                    format!("{truncated}...")
                } else {
                    body.clone()
                };
                entries.push(format!(
                    "## 记忆：{summary}\n- 日期：{date}\n- 标签：{tags}\n\n{body_truncated}"
                ));
            }
        }

        if entries.is_empty() {
            String::new()
        } else {
            entries.join("\n\n")
        }
    }

    /// 按时间戳反向排序列出记忆文件（最新的在前）。
    fn list_memory_files(&self) -> Vec<PathBuf> {
        let dir = self.memory_dir();
        let mut files: Vec<PathBuf> = match fs::read_dir(&dir) {
            Ok(rd) => rd
                .filter_map(|entry| {
                    let e = entry.ok()?;
                    let path = e.path();
                    if path.extension()? == "md" {
                        Some(path)
                    } else {
                        None
                    }
                })
                .collect(),
            Err(_) => return Vec::new(),
        };
        // 文件名即时间戳，按名称反向排序（最新的在前）
        files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        files
    }

    /// 解析记忆文件，返回 (元数据, 正文)。
    fn parse_memory_file(path: &Path) -> Option<(HashMap<String, String>, String)> {
        let content = fs::read_to_string(path).ok()?;
        Self::parse_frontmatter(&content)
    }

    /// 从文件内容解析 frontmatter + 正文。
    fn parse_frontmatter(content: &str) -> Option<(HashMap<String, String>, String)> {
        let rest = content.strip_prefix("---\n")?;
        let mut lines = rest.lines();
        let mut meta = HashMap::new();

        // 读取 frontmatter 行直到下一个 `---`
        for line in &mut lines {
            if line == "---" {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                let val = value.trim().trim_matches('"');
                meta.insert(key.trim().to_string(), val.to_string());
            }
        }

        // 剩余部分为正文（跳过前导空行）
        let body = lines
            .skip_while(|l| l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        Some((meta, body))
    }

    /// 仅解析正文（跳过 frontmatter）。
    fn parse_body(content: &str) -> Option<String> {
        let rest = content.strip_prefix("---\n")?;
        let mut lines = rest.lines();
        // 跳过 frontmatter 行直到 `---`
        for line in &mut lines {
            if line == "---" {
                break;
            }
        }
        let body = lines
            .skip_while(|l| l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        Some(body)
    }
}

#[async_trait]
impl ContextProvider for MemoryManager {
    fn name(&self) -> &str {
        "memory"
    }

    async fn query(&self, _query: &str, _top_k: usize) -> Vec<ContextDocument> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// 创建临时目录用于测试
    fn setup_temp(test_name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("qianxun_test_memory_{test_name}"));
        let _ = fs::remove_dir_all(&dir);
        let ws_root = std::env::temp_dir().join(format!("qianxun_test_ws_{test_name}"));
        let _ = fs::create_dir_all(&ws_root);
        (dir, ws_root)
    }

    fn cleanup(dir: &Path, ws_root: &Path) {
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(ws_root);
    }

    #[test]
    fn test_memory_read_write() {
        let (dir, ws_root) = setup_temp("rw");
        let mm = MemoryManager::new(dir.clone(), &ws_root, 5);

        mm.write_memory("测试摘要", &["test"], "这是正文内容");

        let files = mm.list_memory_files();
        assert_eq!(files.len(), 1, "应该有一个记忆文件");

        // 通过 read 验证正文
        let name = files[0].file_name().unwrap().to_str().unwrap();
        let content = mm.read(name);
        assert!(content.is_some(), "应该能读取到正文");
        assert!(content.unwrap().contains("这是正文内容"));

        cleanup(&dir, &ws_root);
    }

    #[test]
    fn test_memory_build_context() {
        let (dir, ws_root) = setup_temp("ctx");
        let mm = MemoryManager::new(dir.clone(), &ws_root, 3);

        // 写入 5 条记忆，每条间隔 2ms 确保时间戳不同
        for i in 0..5 {
            mm.write_memory(&format!("记忆 {i}"), &["test"], &format!("正文内容 {i}"));
            std::thread::sleep(Duration::from_millis(100));
        }

        let ctx = mm.build_context();
        assert!(!ctx.is_empty(), "build_context 不应为空");

        // 最近的 3 条应包含记忆 2、3、4，不应包含 0、1
        assert!(ctx.contains("记忆 4"), "应包含最近的记忆 4");
        assert!(ctx.contains("记忆 3"), "应包含记忆 3");
        assert!(ctx.contains("记忆 2"), "应包含记忆 2");
        assert!(!ctx.contains("记忆 0"), "不应包含最早的记忆 0");

        // 验证格式化结构
        assert!(ctx.starts_with("## 记忆："), "应以 ## 记忆： 开头");

        cleanup(&dir, &ws_root);
    }

    #[test]
    fn test_memory_persistence() {
        let (dir, ws_root) = setup_temp("persist");
        let ws_root_str = ws_root.to_string_lossy().to_string();

        // 写入一条记忆
        {
            let mm = MemoryManager::new(dir.clone(), Path::new(&ws_root_str), 5);
            mm.write_memory("持久化测试", &["persist"], "应在之后可读");
        } // MemoryManager 被丢弃

        // 用相同目录重建
        {
            let mm = MemoryManager::new(dir.clone(), Path::new(&ws_root_str), 5);
            let files = mm.list_memory_files();
            assert_eq!(files.len(), 1, "重建后应仍有 1 个记忆文件");
            let content = mm.read(files[0].file_name().unwrap().to_str().unwrap());
            assert!(content.is_some(), "应能读取记忆");
            assert!(content.unwrap().contains("应在之后可读"), "内容应持久化");
        }

        cleanup(&dir, &ws_root);
    }

    #[test]
    fn test_memory_empty() {
        let (dir, ws_root) = setup_temp("empty");
        let mm = MemoryManager::new(dir.clone(), &ws_root, 5);

        assert!(mm.build_context().is_empty(), "空记忆目录应返回空字符串");
        assert!(mm.list_memory_files().is_empty(), "空目录应无记忆文件");

        cleanup(&dir, &ws_root);
    }

    #[test]
    fn test_project_hash_deterministic() {
        let (dir, _ws_root) = setup_temp("hash");
        // 创建一个临时路径，不规范化
        let path = std::env::temp_dir().join("qianxun_hash_test");
        let _ = fs::create_dir_all(&path);

        let hash1 = MemoryManager::project_hash(&path);
        let hash2 = MemoryManager::project_hash(&path);
        assert_eq!(hash1, hash2, "相同路径应产生相同哈希");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&path);
    }
}
