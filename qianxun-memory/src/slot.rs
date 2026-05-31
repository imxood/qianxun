use crate::types::{MemorySlot, SlotScope};
use chrono::Utc;
use rusqlite::{params, Connection};

/// 工作记忆插槽管理器。
pub struct SlotManager {
    db: std::sync::Arc<Connection>,
}

impl SlotManager {
    pub fn new(db: std::sync::Arc<Connection>) -> Self {
        Self { db }
    }

    /// 获取所有固定插槽（始终注入到 system prompt）。
    pub fn list_pinned_slots(&self) -> Vec<MemorySlot> {
        let mut stmt = self
            .db
            .prepare("SELECT label, content, size_limit, description, pinned, scope, created_at, updated_at FROM slots WHERE pinned = 1")
            .unwrap();
        stmt.query_map([], |row| {
            let label: String = row.get(0)?;
            let content: String = row.get(1)?;
            let size_limit: usize = row.get(2)?;
            let description: String = row.get(3)?;
            let pinned: i32 = row.get(4)?;
            let scope: String = row.get(5)?;
            let created_at: String = row.get(6)?;
            let updated_at: String = row.get(7)?;
            Ok(MemorySlot {
                label,
                content,
                size_limit,
                description,
                pinned: pinned != 0,
                scope: if scope == "global" { SlotScope::Global } else { SlotScope::Project },
                created_at: parse_rfc3339_or_default(&created_at),
                updated_at: parse_rfc3339_or_default(&updated_at),
            })
        }).unwrap().filter_map(|r| r.ok()).collect()
    }

    /// 追加内容到插槽。
    pub fn append(&self, label: &str, content: &str) -> rusqlite::Result<()> {
        let now = Utc::now().to_rfc3339();
        // 先读现有内容，再追加
        let existing: Option<String> = self
            .db
            .query_row(
                "SELECT content FROM slots WHERE label = ?1",
                params![label],
                |row| row.get(0),
            )
            .ok();

        let new_content = match existing {
            Some(mut old) => {
                old.push('\n');
                old.push_str(content);
                // 截断到 size_limit
                let limit: usize = self
                    .db
                    .query_row(
                        "SELECT size_limit FROM slots WHERE label = ?1",
                        params![label],
                        |row| row.get(0),
                    )
                    .unwrap_or(2000);
                if old.len() > limit {
                    old[old.len() - limit..].to_string()
                } else {
                    old
                }
            }
            None => content.to_string(),
        };

        self.db.execute(
            "INSERT INTO slots (label, content, size_limit, description, pinned, scope, created_at, updated_at)
             VALUES (?1, ?2, 2000, '', 0, 'project', ?3, ?3)
             ON CONFLICT(label) DO UPDATE SET content = ?2, updated_at = ?3",
            params![label, new_content, now],
        )?;
        Ok(())
    }

    /// 替换插槽内容。
    pub fn replace(&self, label: &str, content: &str) -> rusqlite::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO slots (label, content, size_limit, description, pinned, scope, created_at, updated_at)
             VALUES (?1, ?2, 2000, '', 0, 'project', ?3, ?3)
             ON CONFLICT(label) DO UPDATE SET content = ?2, updated_at = ?3",
            params![label, content, now],
        )?;
        Ok(())
    }

    /// 创建插槽。
    pub fn create(&self, label: &str, desc: &str, size_limit: usize) -> rusqlite::Result<()> {
        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO slots (label, content, size_limit, description, pinned, scope, created_at, updated_at)
             VALUES (?1, '', ?2, ?3, 0, 'project', ?4, ?4)",
            params![label, size_limit, desc, now],
        )?;
        Ok(())
    }

    /// 删除插槽。
    pub fn delete(&self, label: &str) -> rusqlite::Result<()> {
        self.db
            .execute("DELETE FROM slots WHERE label = ?1", params![label])?;
        Ok(())
    }
}

/// 安全解析 RFC3339 时间戳，失败时返回 Utc::now()。
fn parse_rfc3339_or_default(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now())
}
