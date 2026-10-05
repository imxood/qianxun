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
    /// 正文首行摘要（跳过标题行，截断 80 字符），列表第二行展示。
    pub excerpt: String,
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
///
/// `async` + `spawn_blocking`（docs/18 C3）：Tauri v2 中非 async 命令在**主线程**
/// 执行，而本命令要遍历整个库并逐篇读文件头——笔记页每次聚焦/保存后都会调它，
/// 1000 篇规模下会周期性卡住 UI。搬到阻塞线程池后主线程不再被占。
#[tauri::command]
pub async fn notes_list(vault: String) -> Result<Vec<NoteMeta>> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = vault_root(&vault)?;
        let mut notes = Vec::new();
        collect_notes(&root, &root, &mut notes)?;
        // 最近修改优先。
        notes.sort_by_key(|note| Reverse(note.updated));
        Ok(notes)
    })
    .await
    .map_err(|cause| Error::Notes(format!("清单扫描任务失败：{cause}")))?
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

/// 原子保存：结构化元数据 + 纯正文，由 Rust 统一拼装 frontmatter。
/// 读（剥）/写（拼）两侧同一实现，杜绝「首存丢元数据」的契约裂缝；
/// 已有的 created 字段原样保留（用户建笔记日期不因编辑而变）。
#[tauri::command]
pub fn notes_save(
    vault: String,
    path: String,
    title: String,
    tags: Vec<String>,
    body: String,
) -> Result<NoteMeta> {
    let file = note_path(&vault, &path)?;
    let created = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| frontmatter_field(&text, "created"));
    let content = compose_frontmatter(&title, &tags, created.as_deref(), &body);
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

/// 清单扫描时读取单篇的上限（docs/18 C3）。
///
/// 摘要只需要 frontmatter + 正文开头几十字节，但旧实现为了算 80 字摘要
/// 把**整篇**读进内存——一篇 5MB 的长笔记就是 5MB 的分配 × N 篇。
/// 超限后只读前 `EXCERPT_READ_CAP` 字节：摘要与 title 的来源都在文件头，
/// 截断不影响清单正确性（正文要读时走 notes_read，那条路不截断）。
const EXCERPT_READ_CAP: u64 = 64 * 1024;

/// 读取清单所需的文件头（最多 `EXCERPT_READ_CAP` 字节）。
fn read_head(path: &Path) -> String {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = Vec::new();
    // 读超量会返回 io::ErrorKind::UnexpectedEof，取已读部分即可。
    let _ = file.by_ref().take(EXCERPT_READ_CAP).read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
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
            let text = read_head(&path);
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
        excerpt: excerpt_of(strip_frontmatter(text)),
        updated,
        size: stat.len(),
    })
}

/// 正文首行摘要：跳过空行与 Markdown 标题行，截断 80 字符。
fn excerpt_of(body: &str) -> String {
    let first = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or_default();
    if first.chars().count() <= 80 {
        first.to_owned()
    } else {
        let head: String = first.chars().take(80).collect();
        format!("{head}…")
    }
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

/// 取 frontmatter 中的单值字段（如 created），不含 title/tags 的通用读取。
fn frontmatter_field(text: &str, key: &str) -> Option<String> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let prefix = format!("{key}:");
    rest[..end].lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .map(|value| value.trim().to_owned())
    })
}

/// 拼装 frontmatter + 正文：与 parse_frontmatter 同源语义（单测互逆）。
/// 单值字段压平换行防止破坏块结构；tags 用逗号分隔的字面列表。
fn compose_frontmatter(title: &str, tags: &[String], created: Option<&str>, body: &str) -> String {
    let flat_title = title.replace(['\n', '\r'], " ");
    let safe_tags: Vec<String> = tags
        .iter()
        .map(|tag| {
            tag.replace(['\n', '\r', ',', '[', ']'], " ")
                .trim()
                .to_owned()
        })
        .filter(|tag| !tag.is_empty())
        .collect();
    let mut front = format!(
        "---\ntitle: {flat_title}\ntags: [{}]\n",
        safe_tags.join(", ")
    );
    if let Some(date) = created {
        front.push_str(&format!("created: {date}\n"));
    }
    front.push_str("---\n\n");
    front.push_str(body);
    front
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
    fn 拼装与解析互逆且保留created() {
        let tags = vec!["rust".to_owned(), "工具".to_owned()];
        let composed = compose_frontmatter("我的笔记", &tags, Some("2026-09-03"), "正文第一行");
        let (title, parsed_tags) = parse_frontmatter(&composed).expect("拼装结果应可解析");
        assert_eq!(title, "我的笔记");
        assert_eq!(parsed_tags, tags);
        assert_eq!(strip_frontmatter(&composed), "正文第一行");
        assert_eq!(
            frontmatter_field(&composed, "created").as_deref(),
            Some("2026-09-03")
        );

        // 换行/逗号等破坏块结构的字符被压平。
        let hostile = compose_frontmatter("a\nb", &["x,y".to_owned()], None, "body");
        assert!(hostile.starts_with("---\ntitle: a b\ntags: [x y]\n---\n\nbody"));
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

    /// docs/18 C3：清单只读文件头，不得把整篇长笔记拉进内存。
    #[test]
    fn 清单只读文件头且仍能取出摘要() {
        let dir = std::env::temp_dir().join("qx-notes-head-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建库目录");
        // 正文远大于上限：模拟一篇长笔记。
        let long_body = "长正文内容。".repeat(40_000); // ≈ 280KB > 64KB
        let content = format!(
            "---\ntitle: 长笔记\ntags: [\"测试\"]\ncreated: 2026-09-03\n---\n\n摘要首行。\n{long_body}"
        );
        let file = dir.join("long.md");
        std::fs::write(&file, &content).expect("写文件");

        let head = read_head(&file);
        assert!(
            head.len() as u64 <= EXCERPT_READ_CAP,
            "读取量必须被上限约束，实际 {} 字节",
            head.len()
        );
        // 关键：摘要与 title 的来源都在文件头，截断不影响清单正确性。
        let meta = meta_from_file(&file, &dir, &head).expect("应解析出元数据");
        assert_eq!(meta.title, "长笔记");
        assert!(meta.excerpt.contains("摘要首行"), "摘要应来自文件头");
        assert!(
            meta.size > EXCERPT_READ_CAP,
            "size 取真实文件大小而非读取量"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 清单扫描跳过隐藏目录与超长读取() {
        let dir = std::env::temp_dir().join("qx-notes-walk-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".trash")).expect("建库目录");
        std::fs::write(dir.join("a.md"), "---\ntitle: A\n---\n\n正文").expect("写 A");
        std::fs::write(dir.join(".trash/deleted.md"), "---\ntitle: 删\n---\n\nx")
            .expect("写回收站");
        std::fs::create_dir_all(dir.join("sub")).expect("建子目录");
        std::fs::write(dir.join("sub/b.md"), "---\ntitle: B\n---\n\n正文").expect("写 B");

        let mut out = Vec::new();
        collect_notes(&dir, &dir, &mut out).expect("遍历应成功");
        let titles: Vec<&str> = out.iter().map(|n| n.title.as_str()).collect();
        assert!(
            titles.contains(&"A") && titles.contains(&"B"),
            "应含 A 与 B"
        );
        assert!(!titles.contains(&"删"), ".trash 必须跳过");
        assert_eq!(out.len(), 2, "只应有 2 篇");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
