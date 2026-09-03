//! 笔记域 IPC 命令：清单 / 读 / 原子写 / 新建 / 删除。

use std::cmp::Reverse;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::atomic;
use crate::error::{Error, Result};

/// 笔记元数据（列表项）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NoteMeta {
    /// 相对于库目录的路径（正斜杠）。
    pub path: String,
    pub title: String,
    pub tags: Vec<String>,
    /// 文件修改时间（毫秒时间戳）。
    pub updated: i64,
    pub size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContent {
    pub meta: NoteMeta,
    pub body: String,
}

/// 列出库内全部 Markdown 笔记（walk 一层子目录即可满足当前需求；深层递归也做）。
#[tauri::command]
pub fn notes_list(vault: String) -> Result<Vec<NoteMeta>> {
    let root = vault_root(&vault)?;
    let mut notes = Vec::new();
    collect_notes(&root, &root, &mut notes)?;
    // 最近修改优先。
    notes.sort_by_key(|note| Reverse(note.updated));
    Ok(notes)
}

/// 读取一篇笔记（正文 + 元数据）。
#[tauri::command]
pub fn notes_read(vault: String, path: String) -> Result<NoteContent> {
    let file = note_path(&vault, &path)?;
    let text = std::fs::read_to_string(&file)
        .map_err(|cause| Error::Notes(format!("读笔记失败：{cause}")))?;
    let meta = meta_from_file(&file, &vault_root(&vault)?, &text)?;
    // 元数据里的正文剥去 frontmatter。
    let body = strip_frontmatter(&text).to_owned();
    Ok(NoteContent { meta, body })
}

/// 原子保存（正文原样写回；frontmatter 由前端编辑器维护在同一文本里）。
#[tauri::command]
pub fn notes_save(vault: String, path: String, content: String) -> Result<NoteMeta> {
    let file = note_path(&vault, &path)?;
    atomic::write(&file, content.as_bytes())
        .map_err(|cause| Error::Notes(format!("写笔记失败：{cause}")))?;
    let text = std::fs::read_to_string(&file)
        .map_err(|cause| Error::Notes(format!("回读笔记失败：{cause}")))?;
    meta_from_file(&file, &vault_root(&vault)?, &text)
}

/// 新建笔记：文件名 = 时间戳 slug（防重），带 frontmatter 模板。返回相对路径。
#[tauri::command]
pub fn notes_create(vault: String, title: String) -> Result<NoteMeta> {
    let root = vault_root(&vault)?;
    let safe_title = sanitize_title(&title);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let file = root.join(format!("{stamp}-{safe_title}.md"));
    let content = format!(
        "---\ntitle: {title}\ntags: []\ncreated: {}\n---\n\n",
        chrono::Local::now().format("%Y-%m-%d")
    );
    atomic::write(&file, content.as_bytes())
        .map_err(|cause| Error::Notes(format!("建笔记失败：{cause}")))?;
    let text = std::fs::read_to_string(&file)
        .map_err(|cause| Error::Notes(format!("回读笔记失败：{cause}")))?;
    meta_from_file(&file, &root, &text)
}

/// 删除笔记（移入库内 .trash/ 而非硬删，误删可救）。
#[tauri::command]
pub fn notes_delete(vault: String, path: String) -> Result<()> {
    let file = note_path(&vault, &path)?;
    let root = vault_root(&vault)?;
    let trash = root.join(".trash");
    std::fs::create_dir_all(&trash)
        .map_err(|cause| Error::Notes(format!("建回收目录失败：{cause}")))?;
    let name = file
        .file_name()
        .map(|text| text.to_string_lossy().into_owned())
        .unwrap_or_else(|| "note.md".to_owned());
    let target = trash.join(format!(
        "{}-{}",
        chrono::Local::now().format("%Y%m%d%H%M%S"),
        name
    ));
    std::fs::rename(&file, &target)
        .map_err(|cause| Error::Notes(format!("删笔记失败：{cause}")))?;
    Ok(())
}

/// 初始化（或校验）笔记库目录：不存在则创建默认位置（文档\千寻笔记）。
/// 显式传目录则用它（用户自选位置）。返回库目录绝对路径。
#[tauri::command]
pub fn notes_init(vault: Option<String>) -> Result<String> {
    let root = match vault {
        Some(text) if !text.trim().is_empty() => PathBuf::from(text.trim()),
        _ => dirs::document_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
            .join("千寻笔记"),
    };
    std::fs::create_dir_all(&root)
        .map_err(|cause| Error::Notes(format!("建笔记库失败：{cause}")))?;
    Ok(root.to_string_lossy().into_owned())
}

// ---- 内部 ----

fn vault_root(vault: &str) -> Result<PathBuf> {
    let root = PathBuf::from(vault);
    if !root.is_dir() {
        return Err(Error::Notes(format!("笔记库目录不存在：{vault}")));
    }
    Ok(root)
}

/// 相对路径 → 库内绝对路径（拒绝越界：`..` 与绝对路径）。
fn note_path(vault: &str, relative: &str) -> Result<PathBuf> {
    let root = vault_root(vault)?;
    if Path::new(relative).is_absolute() || relative.split(['/', '\\']).any(|part| part == "..") {
        return Err(Error::Notes(format!("非法笔记路径：{relative}")));
    }
    let file = root.join(relative);
    if !file.is_file() {
        return Err(Error::Notes(format!("笔记不存在：{relative}")));
    }
    Ok(file)
}

fn collect_notes(root: &Path, dir: &Path, out: &mut Vec<NoteMeta>) -> Result<()> {
    let entries =
        std::fs::read_dir(dir).map_err(|cause| Error::Notes(format!("遍历笔记库失败：{cause}")))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_notes(root, &path, out)?;
        } else if name.to_lowercase().ends_with(".md") {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Ok(meta) = meta_from_file(&path, root, &text) {
                out.push(meta);
            }
        }
    }
    Ok(())
}

/// 从文件 + 文本解析元数据（无 frontmatter 时 title = 文件名）。
fn meta_from_file(file: &Path, root: &Path, text: &str) -> Result<NoteMeta> {
    let relative = file
        .strip_prefix(root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.to_string_lossy().into_owned());
    let stat = std::fs::metadata(file)
        .map_err(|cause| Error::Notes(format!("读文件属性失败：{cause}")))?;
    let updated = stat
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|span| span.as_millis() as i64)
        .unwrap_or(0);
    let (title, tags) = parse_frontmatter(text).unwrap_or_else(|| {
        (
            file.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| relative.clone()),
            Vec::new(),
        )
    });
    Ok(NoteMeta {
        path: relative,
        title,
        tags,
        updated,
        size: stat.len(),
    })
}

/// 轻量 frontmatter 解析：`---` 开头块内的 `title:` 与 `tags: [a, b]`。
fn parse_frontmatter(text: &str) -> Option<(String, Vec<String>)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let block = &rest[..end];
    let mut title = None;
    let mut tags = Vec::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("title:") {
            title = Some(value.trim().trim_matches('"').trim_matches('\'').to_owned());
        } else if let Some(value) = line.strip_prefix("tags:") {
            let inner = value.trim().trim_start_matches('[').trim_end_matches(']');
            tags = inner
                .split(',')
                .map(|tag| tag.trim().trim_matches('"').trim_matches('\'').to_owned())
                .filter(|tag| !tag.is_empty())
                .collect();
        }
    }
    title.map(|text| (text, tags))
}

fn strip_frontmatter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim_start_matches('\n'),
        None => text,
    }
}

/// 文件名安全化：仅保留字母数字/汉字/连字符。
fn sanitize_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "note".to_owned()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter解析标题与标签() {
        let text = "---\ntitle: 我的笔记\ntags: [\"rust\", 工具]\ncreated: 2026-09-03\n---\n\n正文";
        let (title, tags) = parse_frontmatter(text).expect("应解析");
        assert_eq!(title, "我的笔记");
        assert_eq!(tags, vec!["rust".to_owned(), "工具".to_owned()]);
        assert_eq!(strip_frontmatter(text), "正文");

        assert_eq!(strip_frontmatter("无 frontmatter"), "无 frontmatter");
        assert!(parse_frontmatter("no frontmatter").is_none());
    }

    #[test]
    fn 标题安全化() {
        assert_eq!(sanitize_title("Hello World/2026!"), "Hello-World-2026");
        assert_eq!(sanitize_title("///"), "note");
    }

    #[test]
    fn 越界路径被拒绝() {
        let dir = std::env::temp_dir();
        let vault = dir.to_string_lossy().into_owned();
        assert!(note_path(&vault, "../escape.md").is_err());
        assert!(note_path(&vault, "Z:/abs.md").is_err());
        assert!(note_path(&vault, "definitely-missing.md").is_err());
    }
}
