use qianxun_core::context::SearchResult;
use crate::vector::VectorIndex;
use rusqlite::{params, Connection};
use std::sync::Arc;
use std::sync::RwLock;

/// 混合搜索 —— FTS5 全文搜索 + 向量语义搜索。
#[allow(dead_code)] // 权重字段: hybrid ranking 待实现, 留作 Phase 3a 骨架
pub struct HybridSearch {
    db: Arc<Connection>,
    vector: Arc<RwLock<Option<VectorIndex>>>,
    bm25_weight: f64,
    vector_weight: f64,
}

impl HybridSearch {
    pub fn new(db: Arc<Connection>) -> Self {
        Self {
            db,
            vector: Arc::new(RwLock::new(None)),
            bm25_weight: 0.4,
            vector_weight: 0.6,
        }
    }

    /// 设置向量索引。
    pub fn set_vector_index(&self, index: VectorIndex) {
        *self.vector.write().unwrap() = Some(index);
    }

    /// 执行混合搜索。
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        // 1. FTS5 搜索（主路径）
        let fts_results = self.fts_search(query, limit * 2);

        // 2. 向量搜索（可选，预留）
        let _vector_results: Vec<(String, f64)> = Vec::new();

        // 3. RRF 融合
        let mut merged = fts_results;

        // 4. Session 去重（max 3 per session）
        merged = self.deduplicate_by_session(merged);

        // 5. 加载完整数据
        merged.truncate(limit);
        merged
    }

    /// FTS5 全文搜索。
    fn fts_search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        // 对中文查询不做特殊处理，unicode61 tokenizer 会做字符级切分
        let fts_query = query
            .split_whitespace()
            .filter(|w| w.len() > 1)
            .collect::<Vec<_>>()
            .join(" ");

        if fts_query.is_empty() {
            return vec![];
        }

        let mut stmt = match self.db.prepare(
            "SELECT o.id, o.session_id, o.timestamp,
                    json_extract(o.data, '$.title') as title,
                    json_extract(o.data, '$.narrative') as narrative,
                    json_extract(o.data, '$.concepts') as concepts,
                    json_extract(o.data, '$.files') as files,
                    json_extract(o.data, '$.importance') as importance,
                    rank
             FROM obs_fts f
             JOIN observations o ON o.rowid = f.rowid
             WHERE obs_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[memory] FTS5 search failed: {e}");
                return vec![];
            }
        };

        let results: Vec<SearchResult> = stmt
            .query_map(params![fts_query, limit as i64], |row| {
                let id: String = row.get(0)?;
                let session_id: String = row.get(1)?;
                let timestamp: String = row.get(2)?;
                let title: String = row.get(3).unwrap_or_default();
                let narrative: String = row.get(4).unwrap_or_default();
                let concepts_json: String = row.get(5).unwrap_or_else(|_| "[]".into());
                let files_json: String = row.get(6).unwrap_or_else(|_| "[]".into());
                let importance: u8 = row.get(7).unwrap_or(0);
                let score: f64 = row.get(8).unwrap_or(0.0);

                let concepts: Vec<String> =
                    serde_json::from_str(&concepts_json).unwrap_or_default();
                let files: Vec<String> = serde_json::from_str(&files_json).unwrap_or_default();

                Ok(SearchResult {
                    id,
                    session_id,
                    timestamp,
                    title: title.trim_matches('"').to_string(),
                    narrative: narrative.trim_matches('"').to_string(),
                    concepts,
                    files,
                    importance,
                    score,
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        results
    }

    /// 按 Session 去重（保留最多 N 条）。
    fn deduplicate_by_session(&self, results: Vec<SearchResult>) -> Vec<SearchResult> {
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let max_per_session = 3;

        results
            .into_iter()
            .filter(|r| {
                let count = seen.entry(r.session_id.clone()).or_insert(0);
                *count += 1;
                *count <= max_per_session
            })
            .collect()
    }
}
